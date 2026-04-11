pub mod index;
pub mod integration;
pub mod manifest;
pub mod resolver;
pub mod submodule;

use anyhow::Result;
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
