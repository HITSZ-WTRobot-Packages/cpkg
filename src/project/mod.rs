pub mod index;
pub mod integration;
pub mod resolver;
pub mod submodule;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

pub const CURRENT_FORMAT_VERSION: u32 = 1;
pub const MANIFEST_FILENAME: &str = "wtrproject.toml";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WtrProject {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    pub project: ProjectSection,
    #[serde(default)]
    pub dependencies: DependencySection,
    #[serde(default, skip_serializing_if = "IndexSection::is_empty")]
    pub index: IndexSection,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub name: String,
    pub ioc_file: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DependencySection {
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndexSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<String>,
}

impl IndexSection {
    pub fn is_empty(&self) -> bool {
        self.path.is_none() && self.url.is_none() && self.cache_path.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct ProjectInitOptions {
    pub force: bool,
    pub name: Option<String>,
    pub ioc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub managed_repo_count: usize,
    pub resolved_package_count: usize,
    pub direct_dependency_count: usize,
    pub integration_file: PathBuf,
}

fn default_format_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

fn normalize_packages(packages: &mut Vec<String>) {
    let unique = packages
        .iter()
        .filter(|pkg| !pkg.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    *packages = unique.into_iter().collect();
}

fn normalize_manifest(project: &mut WtrProject) {
    normalize_packages(&mut project.dependencies.packages);
}

pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_FILENAME)
}

pub fn load(root: &Path) -> Result<WtrProject> {
    let manifest_path = manifest_path(root);
    let content = fs::read_to_string(&manifest_path).context("failed to read wtrproject.toml")?;
    let mut manifest: WtrProject =
        toml::from_str(&content).context("failed to parse wtrproject.toml")?;
    if manifest.format_version != CURRENT_FORMAT_VERSION {
        anyhow::bail!(
            "unsupported wtrproject.toml format version {}",
            manifest.format_version
        );
    }
    normalize_manifest(&mut manifest);
    Ok(manifest)
}

pub fn save(root: &Path, manifest: &WtrProject) -> Result<()> {
    let mut normalized = manifest.clone();
    normalize_manifest(&mut normalized);
    let content = toml::to_string(&normalized).context("failed to serialize wtrproject.toml")?;
    fs::write(manifest_path(root), content).context("failed to write wtrproject.toml")?;
    Ok(())
}

fn default_project_name(root: &Path) -> Result<String> {
    let canonical_root = root
        .canonicalize()
        .context("failed to resolve project root")?;
    let name = canonical_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("failed to infer project name from current directory"))?;
    Ok(name.to_string())
}

fn detect_ioc_file(root: &Path, requested: Option<&str>) -> Result<String> {
    if let Some(ioc_file) = requested {
        let path = if Path::new(ioc_file).is_absolute() {
            PathBuf::from(ioc_file)
        } else {
            root.join(ioc_file)
        };

        if !path.is_file() {
            anyhow::bail!("IOC file '{}' does not exist", ioc_file);
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("ioc") {
            anyhow::bail!("IOC file '{}' must end with .ioc", ioc_file);
        }
        return Ok(ioc_file.to_string());
    }

    let mut ioc_files = fs::read_dir(root)
        .context("failed to scan current directory")?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ioc"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
        })
        .collect::<Vec<_>>();

    ioc_files.sort();

    match ioc_files.as_slice() {
        [] => anyhow::bail!("no STM32CubeMX .ioc file found in the current directory"),
        [ioc_file] => Ok(ioc_file.clone()),
        _ => anyhow::bail!(
            "multiple .ioc files found ({}); use `cpkg init --ioc <FILE>`",
            ioc_files.join(", ")
        ),
    }
}

pub fn project_ioc_path(root: &Path, manifest: &WtrProject) -> PathBuf {
    let configured = Path::new(&manifest.project.ioc_file);
    if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    }
}

pub fn validate_stm32_project(root: &Path, manifest: &WtrProject) -> Result<()> {
    let ioc_path = project_ioc_path(root, manifest);
    if !ioc_path.is_file() {
        anyhow::bail!(
            "configured IOC file '{}' does not exist",
            manifest.project.ioc_file
        );
    }
    if ioc_path.extension().and_then(|ext| ext.to_str()) != Some("ioc") {
        anyhow::bail!(
            "configured IOC file '{}' must end with .ioc",
            manifest.project.ioc_file
        );
    }
    Ok(())
}

pub fn init(root: &Path, options: ProjectInitOptions) -> Result<WtrProject> {
    let manifest_path = manifest_path(root);
    if manifest_path.exists() && !options.force {
        anyhow::bail!("wtrproject.toml already exists (use --force to overwrite)");
    }

    let manifest = WtrProject {
        format_version: CURRENT_FORMAT_VERSION,
        project: ProjectSection {
            name: options.name.clone().unwrap_or(default_project_name(root)?),
            ioc_file: detect_ioc_file(root, options.ioc.as_deref())?,
        },
        dependencies: DependencySection::default(),
        index: IndexSection::default(),
    };

    save(root, &manifest)?;
    info!(
        "initialized STM32CubeMX project '{}' using {}",
        manifest.project.name, manifest.project.ioc_file
    );
    Ok(manifest)
}

pub fn add(root: &Path, packages: &[String]) -> Result<WtrProject> {
    if packages.is_empty() {
        anyhow::bail!("no packages provided");
    }

    let mut manifest = load(root)?;
    manifest
        .dependencies
        .packages
        .extend(packages.iter().cloned());
    save(root, &manifest)?;
    info!("added {} package(s) to wtrproject.toml", packages.len());
    Ok(manifest)
}

pub fn remove(root: &Path, packages: &[String]) -> Result<WtrProject> {
    if packages.is_empty() {
        anyhow::bail!("no packages provided");
    }

    let mut manifest = load(root)?;
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

    manifest
        .dependencies
        .packages
        .retain(|package| !packages.contains(package));
    save(root, &manifest)?;
    info!("removed {} package(s) from wtrproject.toml", packages.len());
    Ok(manifest)
}

pub fn sync(root: &Path) -> Result<SyncSummary> {
    let manifest = load(root)?;
    validate_stm32_project(root, &manifest)?;

    let index = index::load_for_project(root, &manifest)?;
    let resolved = resolver::resolve(&index, &manifest.dependencies.packages)?;
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

#[cfg(test)]
mod tests {
    use super::{ProjectInitOptions, add, init, load, remove};
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
    fn init_detects_single_ioc_file() {
        let dir = make_temp_dir("project-init");
        fs::write(dir.join("robot.ioc"), "").unwrap();

        let manifest = init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: None,
                ioc: None,
            },
        )
        .unwrap();

        assert_eq!(manifest.project.ioc_file, "robot.ioc");
        assert!(dir.join("wtrproject.toml").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn init_requires_explicit_ioc_when_multiple_exist() {
        let dir = make_temp_dir("project-multi-ioc");
        fs::write(dir.join("robot_a.ioc"), "").unwrap();
        fs::write(dir.join("robot_b.ioc"), "").unwrap();

        let error = init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: None,
                ioc: None,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("multiple .ioc files found"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_and_remove_dependencies_update_manifest() {
        let dir = make_temp_dir("project-add-remove");
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

        add(
            &dir,
            &[
                "MotorDrivers::DJI".to_string(),
                "bsp::CANDriver".to_string(),
            ],
        )
        .unwrap();

        let manifest = load(&dir).unwrap();
        assert_eq!(
            manifest.dependencies.packages,
            vec!["MotorDrivers::DJI", "bsp::CANDriver"]
        );

        remove(&dir, &["bsp::CANDriver".to_string()]).unwrap();
        let manifest = load(&dir).unwrap();
        assert_eq!(manifest.dependencies.packages, vec!["MotorDrivers::DJI"]);
        let _ = fs::remove_dir_all(dir);
    }
}
