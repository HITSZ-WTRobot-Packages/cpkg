use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

use crate::config::{
    GlobalConfig, IndexSourceConfig, cpkg_home_dir, global_config_path, load_global_config,
};

use super::network::{NetworkBatchLogger, run_logged_command_in_batch};
use super::{IndexSection, WtrProject};

pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/HITSZ-WTRobot-Packages/.github/main/cpkg_index.json";
pub const DEFAULT_INDEX_FILENAME: &str = "cpkg_index.json";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
        IndexSource::Local { path, .. } => load_from_path(path),
        IndexSource::Remote {
            url,
            cache_path,
            description,
        } => {
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
        IndexSource::Local { path, .. } => load_from_path(path),
        IndexSource::Remote {
            cache_path,
            description,
            ..
        } => {
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
            if refresh {
                load_source_with_refresh(&source)
            } else {
                load_source_without_refresh(&source)
            }
        }
        SourceSelection::Fallback(sources) => {
            let mut errors = Vec::new();
            for source in sources {
                let result = if refresh {
                    load_source_with_refresh(&source)
                } else {
                    load_source_without_refresh(&source)
                };
                match result {
                    Ok(index) => return Ok(index),
                    Err(error) => {
                        let description = match &source {
                            IndexSource::Local { description, .. } => description,
                            IndexSource::Remote { description, .. } => description,
                        };
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
        fs::write(
            dir.join("cpkg_index.json"),
            r#"{"generated_at":"2026-01-01T00:00:00Z","packages":[]}"#,
        )
        .unwrap();

        let index = load_for_project(&dir, &empty_manifest()).unwrap();
        assert!(index.packages.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_for_project_without_refresh_uses_configured_cache() {
        let dir = make_temp_dir("configured-cache");
        let cache_path = dir.join("cache").join("cpkg_index.json");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(
            &cache_path,
            r#"{"generated_at":"2026-01-01T00:00:00Z","packages":[]}"#,
        )
        .unwrap();

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
        fs::write(
            &second_cache,
            r#"{"generated_at":"2026-01-01T00:00:00Z","packages":[]}"#,
        )
        .unwrap();

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
                generated_at: Some("2026-01-01T00:00:00Z".to_string()),
                packages: Vec::new(),
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn explicit_project_index_overrides_global_sources() {
        let dir = make_temp_dir("project-overrides-global");
        let explicit = dir.join("explicit.json");
        fs::write(
            &explicit,
            r#"{"generated_at":"2026-01-01T00:00:00Z","packages":[]}"#,
        )
        .unwrap();

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
}
