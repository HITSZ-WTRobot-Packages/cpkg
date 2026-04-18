use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::{
    GlobalConfig, IndexSourceConfig, cpkg_home_dir, global_config_path, load_global_config,
};

use super::network::{NetworkBatchLogger, run_logged_command_in_batch};
use super::{IndexSection, WtrProject};

pub const DEFAULT_INDEX_URL: &str = "https://raw.githubusercontent.com/HITSZ-WTRobot-Packages/index/refs/heads/main/cpkg_index.json";
pub const DEFAULT_INDEX_FILENAME: &str = "cpkg_index.json";

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PackageIndex {
    #[serde(default)]
    pub generated_at: Option<String>,
    pub packages: Vec<IndexedPackage>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IndexedPackage {
    pub repo: String,
    pub path: String,
    pub name: String,
    pub pkgname: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawPackageIndex {
    Legacy(LegacyPackageIndex),
    RepositoryMap(BTreeMap<String, Vec<RepositoryIndexedPackage>>),
}

#[derive(Deserialize)]
struct LegacyPackageIndex {
    #[serde(default)]
    generated_at: Option<String>,
    packages: Vec<LegacyIndexedPackage>,
}

#[derive(Deserialize)]
struct LegacyIndexedPackage {
    repo: String,
    path: String,
    name: String,
    pkgname: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct RepositoryIndexedPackage {
    path: String,
    name: String,
    pkgname: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

fn normalize_package_path(repo: &str, path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./").trim_start_matches('/');
    let repo_root = format!("Modules/{repo}");

    if trimmed == repo_root || trimmed.starts_with(&format!("{repo_root}/")) {
        trimmed.to_string()
    } else if trimmed.is_empty() {
        repo_root
    } else {
        format!("{repo_root}/{trimmed}")
    }
}

impl LegacyIndexedPackage {
    fn into_indexed_package(self) -> IndexedPackage {
        IndexedPackage {
            repo: self.repo.clone(),
            path: normalize_package_path(&self.repo, &self.path),
            name: self.name,
            pkgname: self.pkgname,
            version: self.version,
            dependencies: self.dependencies,
        }
    }
}

impl RepositoryIndexedPackage {
    fn into_indexed_package(self, repo: &str) -> IndexedPackage {
        IndexedPackage {
            repo: repo.to_string(),
            path: normalize_package_path(repo, &self.path),
            name: self.name,
            pkgname: self.pkgname,
            version: self.version,
            dependencies: self.dependencies,
        }
    }
}

impl RawPackageIndex {
    fn into_package_index(self) -> PackageIndex {
        match self {
            Self::Legacy(index) => PackageIndex {
                generated_at: index.generated_at,
                packages: index
                    .packages
                    .into_iter()
                    .map(LegacyIndexedPackage::into_indexed_package)
                    .collect(),
            },
            Self::RepositoryMap(repositories) => PackageIndex {
                generated_at: None,
                packages: repositories
                    .into_iter()
                    .flat_map(|(repo, packages)| {
                        packages
                            .into_iter()
                            .map(move |package| package.into_indexed_package(&repo))
                    })
                    .collect(),
            },
        }
    }
}

impl<'de> Deserialize<'de> for PackageIndex {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RawPackageIndex::deserialize(deserializer).map(RawPackageIndex::into_package_index)
    }
}

#[derive(Debug, Clone)]
enum IndexSource {
    Local {
        path: PathBuf,
        description: String,
    },
    Remote {
        url: String,
        cache_path: PathBuf,
        description: String,
    },
}

#[derive(Debug, Clone)]
enum SourceSelection {
    Strict(IndexSource),
    Fallback(Vec<IndexSource>),
}

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn default_cache_path() -> Result<PathBuf> {
    Ok(cpkg_home_dir()?.join(DEFAULT_INDEX_FILENAME))
}

fn stable_source_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn default_cache_path_for_global_url(url: &str) -> Result<PathBuf> {
    Ok(cpkg_home_dir()?
        .join("indexes")
        .join(format!("{:016x}.json", stable_source_hash(url))))
}

fn build_source(
    base_root: &Path,
    path: Option<&str>,
    url: Option<&str>,
    cache_path: Option<&str>,
    default_cache: impl FnOnce() -> Result<PathBuf>,
    description: String,
) -> Result<IndexSource> {
    match (path, url) {
        (Some(_), Some(_)) => anyhow::bail!("{description} cannot set both `path` and `url`"),
        (None, None) => anyhow::bail!("{description} must set either `path` or `url`"),
        (Some(path), None) => {
            if cache_path.is_some() {
                anyhow::bail!("{description} cannot set `cache_path` without `url`");
            }
            Ok(IndexSource::Local {
                path: resolve_path(base_root, path),
                description,
            })
        }
        (None, Some(url)) => {
            let cache_path = if let Some(cache_path) = cache_path {
                resolve_path(base_root, cache_path)
            } else {
                default_cache()?
            };
            Ok(IndexSource::Remote {
                url: url.to_string(),
                cache_path,
                description,
            })
        }
    }
}

fn project_index_source(root: &Path, settings: &IndexSection) -> Result<Option<IndexSource>> {
    if settings.is_empty() {
        return Ok(None);
    }

    Ok(Some(build_source(
        root,
        settings.path.as_deref(),
        settings.url.as_deref(),
        settings.cache_path.as_deref(),
        default_cache_path,
        "project [index]".to_string(),
    )?))
}

fn global_index_source(
    config_dir: &Path,
    source: &IndexSourceConfig,
    index: usize,
) -> Result<IndexSource> {
    let description = match (source.path.as_deref(), source.url.as_deref()) {
        (Some(path), None) => format!("global index source {} ({path})", index + 1),
        (None, Some(url)) => format!("global index source {} ({url})", index + 1),
        _ => format!("global index source {}", index + 1),
    };

    build_source(
        config_dir,
        source.path.as_deref(),
        source.url.as_deref(),
        source.cache_path.as_deref(),
        || {
            let url = source
                .url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("global index source is missing `url`"))?;
            default_cache_path_for_global_url(url)
        },
        description,
    )
}

fn builtin_index_source() -> Result<IndexSource> {
    Ok(IndexSource::Remote {
        url: DEFAULT_INDEX_URL.to_string(),
        cache_path: default_cache_path()?,
        description: "built-in default index".to_string(),
    })
}

fn determine_sources(
    root: &Path,
    manifest: &WtrProject,
    global_config: &GlobalConfig,
    global_config_dir: &Path,
) -> Result<SourceSelection> {
    if let Some(source) = project_index_source(root, &manifest.index)? {
        return Ok(SourceSelection::Strict(source));
    }

    let project_local_index = root.join(DEFAULT_INDEX_FILENAME);
    if project_local_index.exists() {
        return Ok(SourceSelection::Strict(IndexSource::Local {
            path: project_local_index,
            description: "project-local cpkg_index.json".to_string(),
        }));
    }

    let mut sources = global_config
        .index
        .iter()
        .enumerate()
        .map(|(index, source)| global_index_source(global_config_dir, source, index))
        .collect::<Result<Vec<_>>>()?;
    sources.push(builtin_index_source()?);
    Ok(SourceSelection::Fallback(sources))
}

pub fn load_from_path(path: &Path) -> Result<PackageIndex> {
    let content = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read package index from '{}'",
            path.to_string_lossy()
        )
    })?;
    let index = serde_json::from_str::<PackageIndex>(&content).with_context(|| {
        format!(
            "failed to parse package index from '{}'",
            path.to_string_lossy()
        )
    })?;
    Ok(index)
}

fn run_curl_download(url: &str, target: &Path, logger: &NetworkBatchLogger) -> Result<()> {
    let mut command = Command::new("curl");
    command.arg("-fsSL").arg("-o").arg(target).arg(url);
    run_logged_command_in_batch(
        &mut command,
        "Downloading package index with curl",
        "index",
        logger,
    )
    .context("failed to execute curl for package index download")?;
    Ok(())
}

#[cfg(windows)]
fn power_shell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(windows)]
fn run_powershell_download(
    program: &str,
    url: &str,
    target: &Path,
    logger: &NetworkBatchLogger,
) -> Result<()> {
    let url = power_shell_literal(url);
    let target = power_shell_literal(&target.to_string_lossy());
    let command = format!("Invoke-WebRequest -Uri '{url}' -OutFile '{target}'");
    let mut process = Command::new(program);
    process.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &command,
    ]);
    run_logged_command_in_batch(
        &mut process,
        &format!("Downloading package index with {program}"),
        "index",
        logger,
    )
    .with_context(|| format!("failed to execute {program} for package index download"))?;
    Ok(())
}

fn download_index(url: &str, cache_path: &Path) -> Result<()> {
    let network_logger = NetworkBatchLogger::new();
    let parent = cache_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid index cache path"))?;
    fs::create_dir_all(parent).context("failed to create index cache directory")?;

    let temp_path = cache_path.with_extension("download");

    let result = (|| -> Result<()> {
        let mut download_errors = Vec::new();
        if let Err(error) = run_curl_download(url, &temp_path, &network_logger) {
            download_errors.push(error.to_string());

            #[cfg(windows)]
            {
                let mut downloaded = false;
                for program in ["powershell", "pwsh"] {
                    match run_powershell_download(program, url, &temp_path, &network_logger) {
                        Ok(()) => {
                            downloaded = true;
                            break;
                        }
                        Err(error) => download_errors.push(error.to_string()),
                    }
                }

                if !downloaded {
                    anyhow::bail!(
                        "failed to download package index from '{}': {}",
                        url,
                        download_errors.join(" | ")
                    );
                }
            }

            #[cfg(not(windows))]
            {
                anyhow::bail!(
                    "failed to download package index from '{}': {}",
                    url,
                    download_errors.join(" | ")
                );
            }
        }

        fs::rename(&temp_path, cache_path).context("failed to update cached package index")?;
        info!(
            "downloaded package index to {}",
            cache_path.to_string_lossy()
        );
        Ok(())
    })();

    match result {
        Ok(()) => {
            network_logger.finish_success()?;
            Ok(())
        }
        Err(error) => {
            network_logger.finish_failure();
            Err(error)
        }
    }
}

fn load_source_with_refresh(source: &IndexSource) -> Result<PackageIndex> {
    match source {
        IndexSource::Local { path, description } => {
            debug!(
                source = %description,
                path = %path.display(),
                "loading local package index"
            );
            load_from_path(path)
        }
        IndexSource::Remote {
            url,
            cache_path,
            description,
        } => {
            debug!(
                source = %description,
                url = %url,
                cache_path = %cache_path.display(),
                "refreshing remote package index"
            );
            if let Err(error) = download_index(url, cache_path) {
                if cache_path.exists() {
                    warn!(
                        "failed to refresh {} from {}: {}; falling back to cached copy",
                        description, url, error
                    );
                } else {
                    return Err(error);
                }
            }
            load_from_path(cache_path)
        }
    }
}

fn load_source_without_refresh(source: &IndexSource) -> Result<PackageIndex> {
    match source {
        IndexSource::Local { path, description } => {
            debug!(
                source = %description,
                path = %path.display(),
                "loading local package index without refresh"
            );
            load_from_path(path)
        }
        IndexSource::Remote {
            cache_path,
            description,
            ..
        } => {
            debug!(
                source = %description,
                cache_path = %cache_path.display(),
                "loading cached package index"
            );
            if !cache_path.exists() {
                anyhow::bail!(
                    "no cached package index found for {} at '{}'",
                    description,
                    cache_path.to_string_lossy()
                );
            }
            load_from_path(cache_path)
        }
    }
}

fn load_from_selection(selection: SourceSelection, refresh: bool) -> Result<PackageIndex> {
    match selection {
        SourceSelection::Strict(source) => {
            debug!(refresh, "using strict package index source");
            if refresh {
                load_source_with_refresh(&source)
            } else {
                load_source_without_refresh(&source)
            }
        }
        SourceSelection::Fallback(sources) => {
            let mut errors = Vec::new();
            for source in sources {
                let description = match &source {
                    IndexSource::Local { description, .. } => description,
                    IndexSource::Remote { description, .. } => description,
                };
                debug!(refresh, source = %description, "trying package index source");
                let result = if refresh {
                    load_source_with_refresh(&source)
                } else {
                    load_source_without_refresh(&source)
                };
                match result {
                    Ok(index) => return Ok(index),
                    Err(error) => {
                        errors.push(format!("{description}: {error}"));
                    }
                }
            }

            anyhow::bail!(
                "failed to load package index from all configured sources: {}",
                errors.join(" | ")
            );
        }
    }
}

fn default_global_config_dir() -> Result<PathBuf> {
    let path = global_config_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid global config path"))?;
    Ok(dir.to_path_buf())
}

fn load_for_project_with_global_config(
    root: &Path,
    manifest: &WtrProject,
    global_config: &GlobalConfig,
    global_config_dir: &Path,
    refresh: bool,
) -> Result<PackageIndex> {
    let sources = determine_sources(root, manifest, global_config, global_config_dir)?;
    load_from_selection(sources, refresh)
}

pub fn load_for_project(root: &Path, manifest: &WtrProject) -> Result<PackageIndex> {
    let global_config = load_global_config()?;
    let global_config_dir = default_global_config_dir()?;
    load_for_project_with_global_config(root, manifest, &global_config, &global_config_dir, true)
}

pub fn load_for_project_without_refresh(
    root: &Path,
    manifest: &WtrProject,
) -> Result<PackageIndex> {
    let global_config = load_global_config()?;
    let global_config_dir = default_global_config_dir()?;
    load_for_project_with_global_config(root, manifest, &global_config, &global_config_dir, false)
}

#[cfg(test)]
mod tests {
    use super::{
        PackageIndex, load_for_project, load_for_project_with_global_config,
        load_for_project_without_refresh,
    };
    use crate::config::{GlobalConfig, IndexSourceConfig};
    use crate::project::{DependencySection, IndexSection, OrgSection, ProjectSection, WtrProject};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cpkg-index-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn empty_manifest() -> WtrProject {
        WtrProject {
            format_version: 1,
            project: ProjectSection {
                name: "robot".to_string(),
                ioc_file: "robot.ioc".to_string(),
            },
            dependencies: DependencySection::default(),
            index: IndexSection::default(),
            org: OrgSection::default(),
        }
    }

    #[test]
    fn load_for_project_prefers_project_local_index() {
        let dir = make_temp_dir("project-local");
        fs::write(dir.join("cpkg_index.json"), r#"{}"#).unwrap();

        let index = load_for_project(&dir, &empty_manifest()).unwrap();
        assert!(index.packages.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_for_project_without_refresh_uses_configured_cache() {
        let dir = make_temp_dir("configured-cache");
        let cache_path = dir.join("cache").join("cpkg_index.json");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, r#"{}"#).unwrap();

        let manifest = WtrProject {
            index: IndexSection {
                path: None,
                url: Some("https://example.com/cpkg_index.json".to_string()),
                cache_path: Some("cache/cpkg_index.json".to_string()),
            },
            ..empty_manifest()
        };

        let index = load_for_project_without_refresh(&dir, &manifest).unwrap();
        assert!(index.packages.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn global_index_sources_are_tried_in_order_without_refresh() {
        let dir = make_temp_dir("global-order");
        let global_dir = dir.join("global");
        fs::create_dir_all(&global_dir).unwrap();

        let second_cache = global_dir.join("second.json");
        fs::write(&second_cache, r#"{}"#).unwrap();

        let global_config = GlobalConfig {
            index: vec![
                IndexSourceConfig {
                    path: None,
                    url: Some("https://example.com/first.json".to_string()),
                    cache_path: Some("missing.json".to_string()),
                },
                IndexSourceConfig {
                    path: None,
                    url: Some("https://example.com/second.json".to_string()),
                    cache_path: Some("second.json".to_string()),
                },
            ],
            ..GlobalConfig::default()
        };

        let index = load_for_project_with_global_config(
            &dir,
            &empty_manifest(),
            &global_config,
            &global_dir,
            false,
        )
        .unwrap();

        assert_eq!(
            index,
            PackageIndex {
                generated_at: None,
                packages: Vec::new(),
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn explicit_project_index_overrides_global_sources() {
        let dir = make_temp_dir("project-overrides-global");
        let explicit = dir.join("explicit.json");
        fs::write(&explicit, r#"{}"#).unwrap();

        let manifest = WtrProject {
            index: IndexSection {
                path: Some("explicit.json".to_string()),
                ..IndexSection::default()
            },
            ..empty_manifest()
        };

        let global_config = GlobalConfig {
            index: vec![IndexSourceConfig {
                path: Some("missing.json".to_string()),
                ..IndexSourceConfig::default()
            }],
            ..GlobalConfig::default()
        };

        let index =
            load_for_project_with_global_config(&dir, &manifest, &global_config, &dir, false)
                .unwrap();

        assert!(index.packages.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_from_path_supports_repository_map_format() {
        let dir = make_temp_dir("repository-map");
        let path = dir.join("cpkg_index.json");
        fs::write(
            &path,
            r#"{
  "BasicComponents": [
    {
      "path":"bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    }
  ],
  "MotorDrivers": [
    {
      "path":"motors\\DJI",
      "name":"DJI",
      "pkgname":"MotorDrivers::DJI",
      "version":"0.1.0",
      "dependencies":["bsp::CANDriver"]
    }
  ]
}"#,
        )
        .unwrap();

        let index = super::load_from_path(&path).unwrap();

        assert_eq!(index.generated_at, None);
        assert_eq!(index.packages.len(), 2);
        assert!(
            index
                .packages
                .iter()
                .any(|package| package.repo == "BasicComponents"
                    && package.path == "Modules/BasicComponents/bsp/can_driver")
        );
        assert!(
            index
                .packages
                .iter()
                .any(|package| package.repo == "MotorDrivers"
                    && package.path == "Modules/MotorDrivers/motors/DJI")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_from_path_keeps_support_for_legacy_flat_format() {
        let dir = make_temp_dir("legacy-flat");
        let path = dir.join("cpkg_index.json");
        fs::write(
            &path,
            r#"{
  "generated_at":"2026-01-01T00:00:00Z",
  "packages":[
    {
      "repo":"BasicComponents",
      "path":"bsp/can_driver",
      "name":"CANDriver",
      "pkgname":"bsp::CANDriver",
      "version":"0.1.0",
      "dependencies":["stm32cubemx"]
    }
  ]
}"#,
        )
        .unwrap();

        let index = super::load_from_path(&path).unwrap();

        assert_eq!(index.generated_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].repo, "BasicComponents");
        assert_eq!(
            index.packages[0].path,
            "Modules/BasicComponents/bsp/can_driver"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
