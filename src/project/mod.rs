pub mod index;
pub mod integration;
pub mod interactive;
pub mod manifest;
pub mod resolver;
pub mod submodule;

use anyhow::Result;
use console::Term;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tracing::info;

pub use self::manifest::{
    DependencySection, IndexSection, ProjectInitOptions, ProjectSection, WtrProject, add, init,
    load, manifest_path, project_ioc_path, remove, save, validate_stm32_project,
};
pub use self::resolver::SubmoduleProtocol;

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub managed_repo_count: usize,
    pub resolved_package_count: usize,
    pub direct_dependency_count: usize,
    pub integration_file: PathBuf,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOptions {
    pub submodule_protocol: SubmoduleProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DependencyEditSummary {
    added: Vec<String>,
    removed: Vec<String>,
    unchanged: Vec<String>,
}

fn merge_requested_packages(
    interactive_packages: &[String],
    explicit_packages: &[String],
) -> Vec<String> {
    let mut merged = Vec::new();
    for package in interactive_packages.iter().chain(explicit_packages.iter()) {
        if !merged.contains(package) {
            merged.push(package.clone());
        }
    }
    merged
}

fn dependency_edit_summary(previous: &[String], next: &[String]) -> DependencyEditSummary {
    let previous_set = previous.iter().cloned().collect::<BTreeSet<_>>();
    let next_set = next.iter().cloned().collect::<BTreeSet<_>>();

    let added = next
        .iter()
        .filter(|package| !previous_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();
    let removed = previous
        .iter()
        .filter(|package| !next_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();
    let unchanged = next
        .iter()
        .filter(|package| previous_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();

    DependencyEditSummary {
        added,
        removed,
        unchanged,
    }
}

fn format_package_list(packages: &[String]) -> String {
    if packages.is_empty() {
        "(none)".to_string()
    } else {
        packages.join(", ")
    }
}

fn write_add_interactive_summary(previous: &[String], next: &[String]) -> Result<()> {
    let summary = dependency_edit_summary(previous, next);
    let term = Term::stderr();

    term.write_line(&format!(
        "Direct dependency changes: {} added, {} removed, {} unchanged.",
        summary.added.len(),
        summary.removed.len(),
        summary.unchanged.len()
    ))?;
    if !summary.added.is_empty() {
        term.write_line(&format!("Added: {}", format_package_list(&summary.added)))?;
    }
    if !summary.removed.is_empty() {
        term.write_line(&format!(
            "Removed: {}",
            format_package_list(&summary.removed)
        ))?;
    }
    term.write_line(&format!("Current: {}", format_package_list(next)))?;

    Ok(())
}

fn add_then_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    if packages.is_empty() {
        return load(root);
    }

    add(root, packages)?;
    sync(root, options)?;
    load(root)
}

fn replace_dependencies_then_sync(
    root: &Path,
    packages: &[String],
    options: SyncOptions,
) -> Result<WtrProject> {
    let mut manifest = load(root)?;
    manifest.dependencies.packages = packages.to_vec();
    save(root, &manifest)?;
    sync(root, options)?;
    load(root)
}

pub fn sync(root: &Path, options: SyncOptions) -> Result<SyncSummary> {
    let manifest = load(root)?;
    validate_stm32_project(root, &manifest)?;

    let index = index::load_for_project(root, &manifest)?;
    let resolved = resolver::resolve(
        &index,
        &manifest.dependencies.packages,
        options.submodule_protocol,
    )?;
    submodule::sync_repositories(root, &resolved.repositories)?;
    let integration_file = integration::write_integration_file(root, &resolved)?;

    let summary = SyncSummary {
        managed_repo_count: resolved.repositories.len(),
        resolved_package_count: resolved.managed_packages.len(),
        direct_dependency_count: manifest.dependencies.packages.len(),
        integration_file,
    };

    info!(
        "sync finished: {} direct package(s), {} resolved package(s), {} repo(s)",
        summary.direct_dependency_count, summary.resolved_package_count, summary.managed_repo_count
    );
    Ok(summary)
}

pub fn add_and_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    add_then_sync(root, packages, options)
}

pub fn init_interactive(root: &Path, options: ProjectInitOptions) -> Result<WtrProject> {
    let mut manifest = manifest::prepare_init(root, &options)?;
    let index = index::load_for_project(root, &manifest)?;
    match interactive::select_dependencies(&index)? {
        None => Ok(manifest),
        Some(packages) => {
            manifest.dependencies.packages = packages;
            save(root, &manifest)?;
            Ok(manifest)
        }
    }
}

pub fn add_interactive(
    root: &Path,
    explicit_packages: &[String],
    options: SyncOptions,
) -> Result<WtrProject> {
    let manifest = load(root)?;
    let previous_packages = manifest.dependencies.packages.clone();
    let index = index::load_for_project(root, &manifest)?;
    let initially_selected_packages =
        merge_requested_packages(&manifest.dependencies.packages, explicit_packages);
    let interactive_packages = match interactive::select_dependencies_with_initial_selection(
        &index,
        &initially_selected_packages,
    )? {
        Some(packages) => packages,
        None => return Ok(manifest),
    };
    let updated_manifest = replace_dependencies_then_sync(root, &interactive_packages, options)?;
    write_add_interactive_summary(&previous_packages, &interactive_packages)?;
    Ok(updated_manifest)
}

#[cfg(test)]
mod tests {
    use super::{dependency_edit_summary, merge_requested_packages};

    #[test]
    fn merge_requested_packages_preserves_order_and_deduplicates() {
        let merged = merge_requested_packages(
            &[
                "MotorDrivers::DJI".to_string(),
                "bsp::CANDriver".to_string(),
            ],
            &[
                "bsp::CANDriver".to_string(),
                "services::Watchdog".to_string(),
            ],
        );

        assert_eq!(
            merged,
            vec!["MotorDrivers::DJI", "bsp::CANDriver", "services::Watchdog"]
        );
    }

    #[test]
    fn dependency_edit_summary_reports_added_removed_and_unchanged() {
        let summary = dependency_edit_summary(
            &[
                "MotorDrivers::DJI".to_string(),
                "bsp::CANDriver".to_string(),
                "services::Watchdog".to_string(),
            ],
            &[
                "bsp::CANDriver".to_string(),
                "services::Referee".to_string(),
                "services::Watchdog".to_string(),
            ],
        );

        assert_eq!(summary.added, vec!["services::Referee"]);
        assert_eq!(summary.removed, vec!["MotorDrivers::DJI"]);
        assert_eq!(
            summary.unchanged,
            vec!["bsp::CANDriver", "services::Watchdog"]
        );
    }
}
