pub mod index;
pub mod integration;
pub mod interactive;
pub mod manifest;
pub mod network;
pub mod resolver;
pub mod submodule;

use anyhow::{Context, Result};
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

fn init_guidance_lines(manifest: &WtrProject) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Next steps to integrate cpkg into your CMake project:".to_string());

    if manifest.dependencies.packages.is_empty() {
        lines.push("1. Add direct dependencies with `cpkg add <PACKAGE>`.".to_string());
        lines.push(format!(
            "2. Run `cpkg sync` to generate `{}`.",
            integration::GENERATED_CMAKE_PATH
        ));
        lines.push(
            "3. Add `include(cmake/wtr_modules.cmake)` to the root `CMakeLists.txt`.".to_string(),
        );
        lines.push(
            "4. Use `wtr_link_packages(<target>)` for plain linking, or `wtr_link_packages_public(<target>)` for `PUBLIC` linking.".to_string(),
        );
    } else {
        lines.push(format!(
            "1. Run `cpkg sync` to generate `{}`.",
            integration::GENERATED_CMAKE_PATH
        ));
        lines.push(
            "2. Add `include(cmake/wtr_modules.cmake)` to the root `CMakeLists.txt`.".to_string(),
        );
        lines.push(
            "3. Use `wtr_link_packages(<target>)` for plain linking, or `wtr_link_packages_public(<target>)` for `PUBLIC` linking.".to_string(),
        );
    }

    lines
}

pub fn write_init_integration_guidance(manifest: &WtrProject) -> Result<()> {
    let term = Term::stderr();
    for line in init_guidance_lines(manifest) {
        term.write_line(&line)?;
    }
    Ok(())
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

fn is_dependency_validation_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("not found in index") || message.contains("dependency cycle detected")
    })
}

fn resolve_project(
    manifest: &WtrProject,
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<resolver::ResolvedProject> {
    resolver::resolve(
        index,
        &manifest.dependencies.packages,
        options.submodule_protocol,
    )
}

fn resolved_project_from_index(
    manifest: &WtrProject,
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<resolver::ResolvedProject> {
    if manifest.dependencies.packages.is_empty() {
        return Ok(resolver::ResolvedProject {
            direct_targets: Vec::new(),
            external_targets: Vec::new(),
            managed_packages: Vec::new(),
            repositories: Vec::new(),
        });
    }

    resolve_project(manifest, index, options)
}

fn finish_sync(
    root: &Path,
    manifest: &WtrProject,
    resolved: &resolver::ResolvedProject,
) -> Result<SyncSummary> {
    validate_stm32_project(root, manifest)?;
    submodule::sync_repositories(root, &resolved.repositories)?;
    let integration_file = integration::write_integration_file(root, resolved)?;

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

fn update_manifest_then<Update, FollowUp, ShouldRestore>(
    root: &Path,
    update: Update,
    follow_up: FollowUp,
    should_restore: ShouldRestore,
) -> Result<WtrProject>
where
    Update: FnOnce(&mut WtrProject),
    FollowUp: FnOnce(&WtrProject) -> Result<()>,
    ShouldRestore: Fn(&anyhow::Error) -> bool,
{
    let previous_manifest = load(root)?;
    let mut updated_manifest = previous_manifest.clone();
    update(&mut updated_manifest);
    save(root, &updated_manifest)?;
    if let Err(error) = follow_up(&updated_manifest) {
        if should_restore(&error) {
            save(root, &previous_manifest).context(
                "failed to restore previous wtrproject.toml after package validation failed",
            )?;
        }
        return Err(error);
    }
    load(root)
}

fn add_then_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    if packages.is_empty() {
        return load(root);
    }

    update_manifest_then(
        root,
        |manifest| {
            manifest
                .dependencies
                .packages
                .extend(packages.iter().cloned());
        },
        |manifest| {
            let index = index::load_for_project(root, manifest)?;
            let resolved = resolve_project(manifest, &index, options)?;
            finish_sync(root, manifest, &resolved).map(|_| ())
        },
        is_dependency_validation_error,
    )
}

fn apply_interactive_selection(
    root: &Path,
    manifest: &WtrProject,
    selected_packages: &[String],
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<(WtrProject, DependencyEditSummary)> {
    let summary = dependency_edit_summary(&manifest.dependencies.packages, selected_packages);
    let mut updated_manifest = manifest.clone();
    updated_manifest.dependencies.packages = selected_packages.to_vec();
    let resolved = resolved_project_from_index(&updated_manifest, index, options)?;
    save(root, &updated_manifest)?;
    finish_sync(root, &updated_manifest, &resolved)?;
    Ok((load(root)?, summary))
}

pub fn sync(root: &Path, options: SyncOptions) -> Result<SyncSummary> {
    let manifest = load(root)?;
    validate_stm32_project(root, &manifest)?;

    let index = index::load_for_project(root, &manifest)?;
    let resolved = resolve_project(&manifest, &index, options)?;
    finish_sync(root, &manifest, &resolved)
}

pub fn add_and_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    add_then_sync(root, packages, options)
}

pub fn init_interactive(root: &Path, options: ProjectInitOptions) -> Result<Option<WtrProject>> {
    let mut manifest = manifest::prepare_init(root, &options)?;
    let index = index::load_for_project(root, &manifest)?;
    match interactive::select_dependencies(&index)? {
        None => Ok(None),
        Some(packages) => {
            manifest.dependencies.packages = packages;
            save(root, &manifest)?;
            Ok(Some(manifest))
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
    let (updated_manifest, _) =
        apply_interactive_selection(root, &manifest, &interactive_packages, &index, options)?;
    write_add_interactive_summary(&previous_packages, &interactive_packages)?;
    Ok(updated_manifest)
}

#[cfg(test)]
mod tests {
    use super::{
        dependency_edit_summary, init_guidance_lines, is_dependency_validation_error,
        merge_requested_packages, update_manifest_then,
    };
    use crate::project::manifest::CURRENT_FORMAT_VERSION;
    use crate::project::{
        DependencySection, IndexSection, ProjectInitOptions, ProjectSection, WtrProject, init, load,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cpkg-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

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
    fn init_guidance_mentions_add_before_sync_when_dependencies_are_empty() {
        let lines = init_guidance_lines(&WtrProject {
            format_version: CURRENT_FORMAT_VERSION,
            project: ProjectSection {
                name: "demo".to_string(),
                ioc_file: "demo.ioc".to_string(),
            },
            dependencies: DependencySection::default(),
            index: IndexSection::default(),
        });

        assert!(lines.iter().any(|line| line.contains("cpkg add <PACKAGE>")));
        assert!(lines.iter().any(|line| {
            line.contains("wtr_link_packages(<target>)")
                && line.contains("wtr_link_packages_public(<target>)")
        }));
    }

    #[test]
    fn init_guidance_skips_add_hint_when_dependencies_are_present() {
        let lines = init_guidance_lines(&WtrProject {
            format_version: CURRENT_FORMAT_VERSION,
            project: ProjectSection {
                name: "demo".to_string(),
                ioc_file: "demo.ioc".to_string(),
            },
            dependencies: DependencySection {
                packages: vec!["MotorDrivers::DJI".to_string()],
            },
            index: IndexSection::default(),
        });

        assert!(!lines.iter().any(|line| line.contains("cpkg add <PACKAGE>")));
        assert!(lines.iter().any(|line| line.contains("cpkg sync")));
    }

    #[test]
    fn update_manifest_then_persists_changes_before_follow_up_failure() {
        let dir = make_temp_dir("persist-before-follow-up");
        fs::write(dir.join("robot.ioc"), "").unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let error = update_manifest_then(
            &dir,
            |manifest| manifest.dependencies.packages = vec!["MotorDrivers::DJI".to_string()],
            |_| Err(anyhow::anyhow!("simulated network failure")),
            is_dependency_validation_error,
        )
        .unwrap_err();

        assert!(error.to_string().contains("simulated network failure"));

        let manifest = load(&dir).unwrap();
        assert_eq!(manifest.dependencies.packages, vec!["MotorDrivers::DJI"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn update_manifest_then_restores_previous_manifest_after_validation_failure() {
        let dir = make_temp_dir("restore-after-validation");
        fs::write(dir.join("robot.ioc"), "").unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let error = update_manifest_then(
            &dir,
            |manifest| manifest.dependencies.packages = vec!["invalid::Package".to_string()],
            |_| {
                Err(anyhow::anyhow!(
                    "package 'invalid::Package' not found in index"
                ))
            },
            is_dependency_validation_error,
        )
        .unwrap_err();

        assert!(error.to_string().contains("not found in index"));

        let manifest = load(&dir).unwrap();
        assert!(manifest.dependencies.packages.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dependency_validation_error_detects_not_found_messages() {
        assert!(is_dependency_validation_error(&anyhow::anyhow!(
            "package 'invalid::Package' not found in index"
        )));
        assert!(!is_dependency_validation_error(&anyhow::anyhow!(
            "simulated network failure"
        )));
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
