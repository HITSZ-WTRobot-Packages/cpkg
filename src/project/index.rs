use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

use super::{IndexSection, WtrProject};

pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/HITSZ-WTRobot-Packages/.github/main/cpkg_index.json";
pub const DEFAULT_INDEX_FILENAME: &str = "cpkg_index.json";
pub const DEFAULT_CACHE_DIRNAME: &str = ".cpkg";

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

enum IndexSource {
    Local(PathBuf),
    Remote { url: String, cache_path: PathBuf },
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("failed to resolve HOME directory"))
}

fn default_cache_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(DEFAULT_CACHE_DIRNAME)
        .join(DEFAULT_INDEX_FILENAME))
}

fn determine_source(root: &Path, settings: &IndexSection) -> Result<IndexSource> {
    if let Some(path) = &settings.path {
        return Ok(IndexSource::Local(resolve_path(root, path)));
    }

    let project_local_index = root.join(DEFAULT_INDEX_FILENAME);
    if project_local_index.exists() {
        return Ok(IndexSource::Local(project_local_index));
    }

    let url = settings
        .url
        .clone()
        .unwrap_or_else(|| DEFAULT_INDEX_URL.to_string());
    let cache_path = if let Some(path) = settings.cache_path.as_deref() {
        resolve_path(root, path)
    } else {
        default_cache_path()?
    };
    Ok(IndexSource::Remote { url, cache_path })
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

fn download_index(url: &str, cache_path: &Path) -> Result<()> {
    let parent = cache_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid index cache path"))?;
    fs::create_dir_all(parent).context("failed to create index cache directory")?;

    let temp_path = cache_path.with_extension("download");
    let output = Command::new("curl")
        .arg("-fsSL")
        .arg("-o")
        .arg(&temp_path)
        .arg(url)
        .output()
        .context("failed to execute curl for package index download")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "failed to download package index from '{}': {}",
            url,
            if stderr.is_empty() {
                "curl exited with an error".to_string()
            } else {
                stderr
            }
        );
    }

    fs::rename(&temp_path, cache_path).context("failed to update cached package index")?;
    info!(
        "downloaded package index to {}",
        cache_path.to_string_lossy()
    );
    Ok(())
}

pub fn load_for_project(root: &Path, manifest: &WtrProject) -> Result<PackageIndex> {
    match determine_source(root, &manifest.index)? {
        IndexSource::Local(path) => load_from_path(&path),
        IndexSource::Remote { url, cache_path } => {
            if let Err(error) = download_index(&url, &cache_path) {
                if cache_path.exists() {
                    warn!(
                        "failed to refresh package index from {}: {}; falling back to cached copy",
                        url, error
                    );
                } else {
                    return Err(error);
                }
            }
            load_from_path(&cache_path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load_for_project;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::project::{DependencySection, IndexSection, ProjectSection, WtrProject};

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
    fn load_for_project_prefers_project_local_index() {
        let dir = make_temp_dir("index-local");
        fs::write(
            dir.join("cpkg_index.json"),
            r#"{"generated_at":"2026-01-01T00:00:00Z","packages":[]}"#,
        )
        .unwrap();

        let manifest = WtrProject {
            format_version: 1,
            project: ProjectSection {
                name: "robot".to_string(),
                ioc_file: "robot.ioc".to_string(),
            },
            dependencies: DependencySection::default(),
            index: IndexSection::default(),
        };

        let index = load_for_project(&dir, &manifest).unwrap();
        assert!(index.packages.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
