use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::config;

use super::feedback::{write_add_interactive_summary, write_add_sync_deferred_notice};
use super::source;
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
    pub submodule_protocol: Option<SubmoduleProtocol>,
    pub offline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeferredAddSync {
    repository_name: String,
}

fn deferred_add_sync(error: &anyhow::Error) -> Option<DeferredAddSync> {
    submodule::online_sync_required_repository(error).map(|repository_name| DeferredAddSync {
        repository_name: repository_name.to_string(),
    })
}

fn sync_command_hint(options: SyncOptions) -> &'static str {
    match options.submodule_protocol {
        Some(SubmoduleProtocol::Https) => "cpkg sync --submodule-protocol https",
        Some(SubmoduleProtocol::Ssh) | None => "cpkg sync",
    }
}

fn finalize_add_result(
    root: &Path,
    result: Result<WtrProject>,
) -> Result<(WtrProject, Option<DeferredAddSync>)> {
    match result {
        Ok(manifest) => Ok((manifest, None)),
        Err(error) => match deferred_add_sync(&error) {
            Some(deferred_sync) => Ok((load(root)?, Some(deferred_sync))),
            None => Err(error),
        },
    }
}

fn write_deferred_add_sync_notice(
    deferred_sync: &DeferredAddSync,
    options: SyncOptions,
) -> Result<()> {
    write_add_sync_deferred_notice(&deferred_sync.repository_name, sync_command_hint(options))
}

fn resolve_project(
    manifest: &WtrProject,
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<ResolvedProject> {
    debug!(
        direct_dependency_count = manifest.dependencies.packages.len(),
        indexed_package_count = index.packages.len(),
        offline = options.offline,
        submodule_protocol = ?options.submodule_protocol,
        "resolving project dependency graph"
    );
    let global_config = config::load_global_config()?;
    let org_source = source::resolve_org_source(manifest, &global_config)?;
    let protocol = source::effective_protocol(&org_source, options.submodule_protocol)?;
    resolver::resolve(
        index,
        &manifest.dependencies.packages,
        &org_source.repository_bases,
        protocol,
    )
}

fn load_index_for_sync(
    root: &Path,
    manifest: &WtrProject,
    options: SyncOptions,
) -> Result<index::PackageIndex> {
    debug!(
        offline = options.offline,
        direct_dependency_count = manifest.dependencies.packages.len(),
        root = %root.display(),
        "loading package index for project"
    );
    if options.offline {
        index::load_for_project_without_refresh(root, manifest)
    } else {
        index::load_for_project(root, manifest)
    }
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
    options: SyncOptions,
) -> Result<SyncSummary> {
    debug!(
        root = %root.display(),
        direct_dependency_count = manifest.dependencies.packages.len(),
        resolved_package_count = resolved.managed_packages.len(),
        repository_count = resolved.repositories.len(),
        offline = options.offline,
        "applying synchronized dependency state"
    );
    let previous_repositories = integration::read_managed_repositories(root)?;
    validate_stm32_project(root, manifest)?;
    submodule::sync_repositories_with_options(root, &resolved.repositories, options.offline)?;
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
    debug!(
        root = %root.display(),
        direct_dependency_count = manifest.dependencies.packages.len(),
        offline = options.offline,
        "refreshing generated integration links"
    );
    let previous_repositories = integration::read_managed_repositories(root)?;
    let resolved = resolved_project_for_integration(root, manifest, options)?;
    let path = integration::write_integration_file(root, &resolved)?;
    submodule::remove_unused_repositories(root, &previous_repositories, &resolved.repositories)?;
    Ok(path)
}

fn add_then_sync(
    root: &Path,
    packages: &[String],
    options: SyncOptions,
) -> Result<(WtrProject, Option<DeferredAddSync>)> {
    debug!(
        package_count = packages.len(),
        offline = options.offline,
        submodule_protocol = ?options.submodule_protocol,
        "adding packages and synchronizing project"
    );
    if packages.is_empty() {
        return Ok((load(root)?, None));
    }

    finalize_add_result(
        root,
        update_manifest_then(
            root,
            |manifest| {
                manifest
                    .dependencies
                    .packages
                    .extend(packages.iter().cloned());
            },
            |manifest| {
                let index = load_index_for_sync(root, manifest, options)?;
                let resolved = resolve_project(manifest, &index, options)?;
                finish_sync(root, manifest, &resolved, options).map(|_| ())
            },
            is_dependency_validation_error,
        ),
    )
}

fn apply_interactive_selection(
    root: &Path,
    manifest: &WtrProject,
    selected_packages: &[String],
    index: &index::PackageIndex,
    options: SyncOptions,
) -> Result<(WtrProject, DependencyEditSummary, Option<DeferredAddSync>)> {
    debug!(
        previous_dependency_count = manifest.dependencies.packages.len(),
        selected_dependency_count = selected_packages.len(),
        indexed_package_count = index.packages.len(),
        offline = options.offline,
        "applying interactive dependency selection"
    );
    let summary =
        super::updates::dependency_edit_summary(&manifest.dependencies.packages, selected_packages);
    let mut updated_manifest = manifest.clone();
    updated_manifest.dependencies.packages = selected_packages.to_vec();
    let resolved = resolved_project_from_index(&updated_manifest, index, options)?;
    save(root, &updated_manifest)?;

    let apply_result = if summary.added.is_empty() {
        refresh_project_links(root, &updated_manifest, options).and_then(|_| load(root))
    } else {
        finish_sync(root, &updated_manifest, &resolved, options).and_then(|_| load(root))
    };
    let (updated_manifest, deferred_sync) = finalize_add_result(root, apply_result)?;

    Ok((updated_manifest, summary, deferred_sync))
}

pub fn sync(root: &Path, options: SyncOptions) -> Result<SyncSummary> {
    debug!(
        root = %root.display(),
        offline = options.offline,
        submodule_protocol = ?options.submodule_protocol,
        "starting project sync"
    );
    let manifest = load(root)?;
    validate_stm32_project(root, &manifest)?;

    let index = load_index_for_sync(root, &manifest, options)?;
    let resolved = resolve_project(&manifest, &index, options)?;
    finish_sync(root, &manifest, &resolved, options)
}

pub fn add_and_sync(root: &Path, packages: &[String], options: SyncOptions) -> Result<WtrProject> {
    debug!(
        root = %root.display(),
        package_count = packages.len(),
        offline = options.offline,
        submodule_protocol = ?options.submodule_protocol,
        "starting add-and-sync workflow"
    );
    let (manifest, deferred_sync) = add_then_sync(root, packages, options)?;
    if let Some(deferred_sync) = deferred_sync {
        write_deferred_add_sync_notice(&deferred_sync, options)?;
    }
    Ok(manifest)
}

pub fn remove(root: &Path, packages: &[String]) -> Result<WtrProject> {
    debug!(
        root = %root.display(),
        package_count = packages.len(),
        "starting package removal workflow"
    );
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
    debug!(
        root = %root.display(),
        force = options.force,
        has_explicit_name = options.name.is_some(),
        has_explicit_ioc = options.ioc.is_some(),
        "starting interactive project init"
    );
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
    debug!(
        root = %root.display(),
        explicit_package_count = explicit_packages.len(),
        offline = options.offline,
        submodule_protocol = ?options.submodule_protocol,
        "starting interactive add workflow"
    );
    let manifest = load(root)?;
    let previous_packages = manifest.dependencies.packages.clone();
    let index = load_index_for_sync(root, &manifest, options)?;
    let initially_selected_packages =
        merge_requested_packages(&manifest.dependencies.packages, explicit_packages);
    let interactive_packages = match interactive::select_dependencies_with_initial_selection(
        &index,
        &initially_selected_packages,
    )? {
        Some(packages) => packages,
        None => return Ok(manifest),
    };
    let (updated_manifest, _, deferred_sync) =
        apply_interactive_selection(root, &manifest, &interactive_packages, &index, options)?;
    write_add_interactive_summary(&previous_packages, &interactive_packages)?;
    if let Some(deferred_sync) = deferred_sync {
        write_deferred_add_sync_notice(&deferred_sync, options)?;
    }
    Ok(updated_manifest)
}

#[cfg(test)]
mod tests {
    use super::{
        SyncOptions, add_then_sync, apply_interactive_selection, finalize_add_result,
        refresh_project_links, remove,
    };
    use crate::project::{ProjectInitOptions, index::PackageIndex, init, load, save, submodule};
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
  "BasicComponents":[
    {
      "path":"bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    }
  ],
  "MotorDrivers":[
    {
      "path":"motors/DJI",
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
  "BasicComponents":[
    {
      "path":"bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    }
  ],
  "MotorDrivers":[
    {
      "path":"motors/DJI",
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

        let (updated_manifest, summary, deferred_sync) = apply_interactive_selection(
            &dir,
            &manifest,
            &["bsp::CANDriver".to_string()],
            &sample_index(),
            SyncOptions::default(),
        )
        .unwrap();

        assert!(summary.added.is_empty());
        assert!(deferred_sync.is_none());
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
  "SharedRepo":[
    {
      "path":"core",
      "name":"Core",
      "pkgname":"SharedRepo::Core",
      "version":"0.1.0",
      "dependencies":[]
    },
    {
      "path":"feature_a",
      "name":"FeatureA",
      "pkgname":"SharedRepo::FeatureA",
      "version":"0.1.0",
      "dependencies":["SharedRepo::Core"]
    },
    {
      "path":"feature_b",
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

    #[test]
    fn finalize_add_result_keeps_manifest_when_online_sync_is_required() {
        let dir = make_temp_dir("deferred-add-result");
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
        manifest.dependencies.packages = vec!["utils::Logger".to_string()];
        save(&dir, &manifest).unwrap();

        let (updated_manifest, deferred_sync) = finalize_add_result(
            &dir,
            Err(submodule::online_sync_required_error("BasicComponents")),
        )
        .unwrap();

        assert_eq!(
            updated_manifest.dependencies.packages,
            vec!["utils::Logger"]
        );
        assert_eq!(
            deferred_sync.unwrap().repository_name,
            "BasicComponents".to_string()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_then_sync_returns_manifest_when_no_packages_are_given() {
        let dir = make_temp_dir("add-no-packages");
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

        let (manifest, deferred_sync) = add_then_sync(&dir, &[], SyncOptions::default()).unwrap();

        assert!(manifest.dependencies.packages.is_empty());
        assert!(deferred_sync.is_none());

        let _ = fs::remove_dir_all(dir);
    }
}
