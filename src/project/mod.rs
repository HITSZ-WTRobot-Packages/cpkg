pub mod index;
pub mod integration;
pub mod interactive;
pub mod manifest;
pub mod resolver;
pub mod submodule;

use anyhow::Result;
use std::io::{BufRead, Write};
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

fn add_then_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    if packages.is_empty() {
        return load(root);
    }

    add(root, packages)?;
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

pub fn init_interactive<R: BufRead, W: Write>(
    root: &Path,
    options: ProjectInitOptions,
    input: &mut R,
    output: &mut W,
) -> Result<WtrProject> {
    let manifest = init(root, options)?;
    let index = index::load_for_project(root, &manifest)?;
    let packages = interactive::select_dependencies(input, output, &index)?;
    if packages.is_empty() {
        Ok(manifest)
    } else {
        add(root, &packages)
    }
}

pub fn add_interactive<R: BufRead, W: Write>(
    root: &Path,
    explicit_packages: &[String],
    options: SyncOptions,
    input: &mut R,
    output: &mut W,
) -> Result<WtrProject> {
    let manifest = load(root)?;
    let index = index::load_for_project(root, &manifest)?;
    let interactive_packages = interactive::select_dependencies(input, output, &index)?;
    let packages = merge_requested_packages(&interactive_packages, explicit_packages);
    add_then_sync(root, &packages, options)
}

#[cfg(test)]
mod tests {
    use super::merge_requested_packages;

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
}
