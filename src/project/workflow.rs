use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::info;

use super::feedback::write_add_interactive_summary;
use super::updates::{
    DependencyEditSummary, is_dependency_validation_error, merge_requested_packages,
    update_manifest_then,
};
use super::{
    index, integration, interactive,
    manifest::{self, ProjectInitOptions, WtrProject, load, save, validate_stm32_project},
    resolver::{self, ResolvedProject, SubmoduleProtocol},
    submodule,
};

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

fn resolve_project(
    manifest: &WtrProject,
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<ResolvedProject> {
    resolver::resolve(
        index,
        &manifest.dependencies.packages,
        options.submodule_protocol,
    )
}

fn empty_resolved_project() -> ResolvedProject {
    ResolvedProject {
        direct_targets: Vec::new(),
        external_targets: Vec::new(),
        managed_packages: Vec::new(),
        repositories: Vec::new(),
    }
}

fn resolved_project_from_index(
    manifest: &WtrProject,
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<ResolvedProject> {
    if manifest.dependencies.packages.is_empty() {
        return Ok(empty_resolved_project());
    }

    resolve_project(manifest, index, options)
}

fn resolved_project_for_integration(
    root: &Path,
    manifest: &WtrProject,
    options: SyncOptions,
) -> Result<ResolvedProject> {
    let index = index::load_for_project_without_refresh(root, manifest)?;
    resolved_project_from_index(manifest, &index, options)
}

fn finish_sync(
    root: &Path,
    manifest: &WtrProject,
    resolved: &ResolvedProject,
) -> Result<SyncSummary> {
    let previous_repositories = integration::read_managed_repositories(root)?;
    validate_stm32_project(root, manifest)?;
    submodule::sync_repositories(root, &resolved.repositories)?;
    let integration_file = integration::write_integration_file(root, resolved)?;
    submodule::remove_unused_repositories(root, &previous_repositories, &resolved.repositories)?;

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

fn refresh_project_links(
    root: &Path,
    manifest: &WtrProject,
    options: SyncOptions,
) -> Result<PathBuf> {
    let previous_repositories = integration::read_managed_repositories(root)?;
    let resolved = resolved_project_for_integration(root, manifest, options)?;
    let path = integration::write_integration_file(root, &resolved)?;
    submodule::remove_unused_repositories(root, &previous_repositories, &resolved.repositories)?;
    Ok(path)
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

pub(crate) fn apply_interactive_selection(
    root: &Path,
    manifest: &WtrProject,
    selected_packages: &[String],
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<(WtrProject, DependencyEditSummary)> {
    let summary =
        super::updates::dependency_edit_summary(&manifest.dependencies.packages, selected_packages);
    let mut updated_manifest = manifest.clone();
    updated_manifest.dependencies.packages = selected_packages.to_vec();
    let resolved = resolved_project_from_index(&updated_manifest, index, options)?;
    save(root, &updated_manifest)?;

    if summary.added.is_empty() {
        refresh_project_links(root, &updated_manifest, options)?;
    } else {
        finish_sync(root, &updated_manifest, &resolved)?;
    }

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

pub fn remove(root: &Path, packages: &[String]) -> Result<WtrProject> {
    if packages.is_empty() {
        anyhow::bail!("no packages provided");
    }

    let manifest = load(root)?;
    let missing = packages
        .iter()
        .filter(|package| !manifest.dependencies.packages.contains(package))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        anyhow::bail!(
            "package(s) not found in wtrproject.toml: {}",
            missing.join(", ")
        );
    }

    update_manifest_then(
        root,
        |manifest| {
            manifest
                .dependencies
                .packages
                .retain(|package| !packages.contains(package));
        },
        |manifest| {
            refresh_project_links(root, manifest, SyncOptions::default())?;
            Ok(())
        },
        |_| true,
    )
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
    use super::{SyncOptions, apply_interactive_selection, refresh_project_links, remove};
    use crate::project::{ProjectInitOptions, index::PackageIndex, init, load, save};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
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

    fn init_git_repo(path: &Path) {
        let status = Command::new("git").arg("init").arg(path).status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn remove_updates_generated_links_without_syncing_modules() {
        let dir = make_temp_dir("remove-refreshes-links");
        init_git_repo(&dir);
        fs::write(dir.join("robot.ioc"), "").unwrap();
        fs::write(
            dir.join("cpkg_index.json"),
            r#"{
  "generated_at":"2026-01-01T00:00:00Z",
  "packages":[
    {
      "repo":"BasicComponents",
      "path":"Modules/BasicComponents/bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    },
    {
      "repo":"MotorDrivers",
      "path":"Modules/MotorDrivers/motors/DJI",
      "name":"DJI",
      "pkgname":"MotorDrivers::DJI",
      "version":"0.1.0",
      "dependencies":["bsp::CANDriver"]
    }
  ]
}"#,
        )
        .unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let mut manifest = load(&dir).unwrap();
        manifest.dependencies.packages = vec![
            "MotorDrivers::DJI".to_string(),
            "bsp::CANDriver".to_string(),
        ];
        save(&dir, &manifest).unwrap();
        let integration_path =
            refresh_project_links(&dir, &manifest, SyncOptions::default()).unwrap();
        let before = fs::read_to_string(&integration_path).unwrap();
        assert!(before.contains("MotorDrivers::DJI"));

        let manifest = remove(&dir, &["MotorDrivers::DJI".to_string()]).unwrap();
        assert_eq!(manifest.dependencies.packages, vec!["bsp::CANDriver"]);

        let after = fs::read_to_string(&integration_path).unwrap();
        assert!(after.contains("set(WTR_DIRECT_PACKAGE_TARGETS\n    bsp::CANDriver\n)"));
        assert!(!after.contains("set(WTR_DIRECT_PACKAGE_TARGETS\n    MotorDrivers::DJI"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_last_package_writes_empty_integration_without_index() {
        let dir = make_temp_dir("remove-last-package");
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

        let mut manifest = load(&dir).unwrap();
        manifest.dependencies.packages = vec!["MotorDrivers::DJI".to_string()];
        save(&dir, &manifest).unwrap();

        let manifest = remove(&dir, &["MotorDrivers::DJI".to_string()]).unwrap();
        assert!(manifest.dependencies.packages.is_empty());

        let integration = fs::read_to_string(dir.join("cmake/wtr_modules.cmake")).unwrap();
        assert!(integration.contains("set(WTR_DIRECT_PACKAGE_TARGETS)"));
        let _ = fs::remove_dir_all(dir);
    }

    fn sample_index_json() -> &'static str {
        r#"{
  "generated_at":"2026-01-01T00:00:00Z",
  "packages":[
    {
      "repo":"BasicComponents",
      "path":"Modules/BasicComponents/bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    },
    {
      "repo":"MotorDrivers",
      "path":"Modules/MotorDrivers/motors/DJI",
      "name":"DJI",
      "pkgname":"MotorDrivers::DJI",
      "version":"0.1.0",
      "dependencies":["bsp::CANDriver"]
    }
  ]
}"#
    }

    fn sample_index() -> PackageIndex {
        serde_json::from_str(sample_index_json()).unwrap()
    }

    #[test]
    fn add_interactive_removal_only_refreshes_links_without_sync() {
        let dir = make_temp_dir("interactive-removal-only");
        init_git_repo(&dir);
        fs::write(dir.join("robot.ioc"), "").unwrap();
        fs::write(dir.join("cpkg_index.json"), sample_index_json()).unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let mut manifest = load(&dir).unwrap();
        manifest.dependencies.packages = vec![
            "MotorDrivers::DJI".to_string(),
            "bsp::CANDriver".to_string(),
        ];
        save(&dir, &manifest).unwrap();
        refresh_project_links(&dir, &manifest, SyncOptions::default()).unwrap();

        let (updated_manifest, summary) = apply_interactive_selection(
            &dir,
            &manifest,
            &["bsp::CANDriver".to_string()],
            &sample_index(),
            SyncOptions::default(),
        )
        .unwrap();

        assert!(summary.added.is_empty());
        assert_eq!(summary.removed, vec!["MotorDrivers::DJI"]);
        assert_eq!(
            updated_manifest.dependencies.packages,
            vec!["bsp::CANDriver"]
        );

        let integration = fs::read_to_string(dir.join("cmake/wtr_modules.cmake")).unwrap();
        assert!(integration.contains("set(WTR_DIRECT_PACKAGE_TARGETS\n    bsp::CANDriver\n)"));
        assert!(!integration.contains("set(WTR_DIRECT_PACKAGE_TARGETS\n    MotorDrivers::DJI"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refresh_project_links_only_adds_package_dirs_in_dependency_chain() {
        let dir = make_temp_dir("refresh-package-subdirs");
        fs::write(dir.join("robot.ioc"), "").unwrap();
        fs::write(
            dir.join("cpkg_index.json"),
            r#"{
  "generated_at":"2026-01-01T00:00:00Z",
  "packages":[
    {
      "repo":"SharedRepo",
      "path":"Modules/SharedRepo/core",
      "name":"Core",
      "pkgname":"SharedRepo::Core",
      "version":"0.1.0",
      "dependencies":[]
    },
    {
      "repo":"SharedRepo",
      "path":"Modules/SharedRepo/feature_a",
      "name":"FeatureA",
      "pkgname":"SharedRepo::FeatureA",
      "version":"0.1.0",
      "dependencies":["SharedRepo::Core"]
    },
    {
      "repo":"SharedRepo",
      "path":"Modules/SharedRepo/feature_b",
      "name":"FeatureB",
      "pkgname":"SharedRepo::FeatureB",
      "version":"0.1.0",
      "dependencies":[]
    }
  ]
}"#,
        )
        .unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let mut manifest = load(&dir).unwrap();
        manifest.dependencies.packages = vec!["SharedRepo::FeatureA".to_string()];
        save(&dir, &manifest).unwrap();

        let integration_path =
            refresh_project_links(&dir, &manifest, SyncOptions::default()).unwrap();
        let integration = fs::read_to_string(&integration_path).unwrap();

        assert!(integration.contains("set(WTR_MANAGED_REPOSITORIES\n    SharedRepo\n)"));
        assert!(integration.contains("Modules/SharedRepo/core"));
        assert!(integration.contains("Modules/SharedRepo/feature_a"));
        assert!(!integration.contains("Modules/SharedRepo/feature_b"));
        assert!(integration.contains("\"${CMAKE_CURRENT_LIST_DIR}/../${_wtr_package_dir}\""));
        assert!(!integration.contains("\"${CMAKE_CURRENT_LIST_DIR}/../Modules/${_wtr_repo}\""));

        let _ = fs::remove_dir_all(dir);
    }
}
