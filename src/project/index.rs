use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::OsString;
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
    home_dir_from_env(
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
        env::var_os("HOMEDRIVE"),
        env::var_os("HOMEPATH"),
    )
    .ok_or_else(|| anyhow::anyhow!("failed to resolve user home directory"))
}

fn home_dir_from_env(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    homedrive: Option<OsString>,
    homepath: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(home) = home {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }

    if let Some(userprofile) = userprofile {
        if !userprofile.is_empty() {
            return Some(PathBuf::from(userprofile));
        }
    }

    match (homedrive, homepath) {
        (Some(homedrive), Some(homepath)) if !homedrive.is_empty() && !homepath.is_empty() => {
            Some(PathBuf::from(format!(
                "{}{}",
                homedrive.to_string_lossy(),
                homepath.to_string_lossy()
            )))
        }
        _ => None,
    }
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

fn run_curl_download(url: &str, target: &Path) -> Result<()> {
    let output = Command::new("curl")
        .arg("-fsSL")
        .arg("-o")
        .arg(target)
        .arg(url)
        .output()
        .context("failed to execute curl for package index download")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "curl failed: {}",
        if stderr.is_empty() {
            "unknown curl error".to_string()
        } else {
            stderr
        }
    );
}

#[cfg(windows)]
fn power_shell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(windows)]
fn run_powershell_download(program: &str, url: &str, target: &Path) -> Result<()> {
    let url = power_shell_literal(url);
    let target = power_shell_literal(&target.to_string_lossy());
    let command = format!("Invoke-WebRequest -Uri '{url}' -OutFile '{target}'");
    let output = Command::new(program)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .output()
        .with_context(|| format!("failed to execute {program} for package index download"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "{program} failed: {}",
        if stderr.is_empty() {
            "unknown PowerShell error".to_string()
        } else {
            stderr
        }
    );
}

fn download_index(url: &str, cache_path: &Path) -> Result<()> {
    let parent = cache_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid index cache path"))?;
    fs::create_dir_all(parent).context("failed to create index cache directory")?;

    let temp_path = cache_path.with_extension("download");

    let mut download_errors = Vec::new();
    if let Err(error) = run_curl_download(url, &temp_path) {
        download_errors.push(error.to_string());

        #[cfg(windows)]
        {
            let mut downloaded = false;
            for program in ["powershell", "pwsh"] {
                match run_powershell_download(program, url, &temp_path) {
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
    use super::{home_dir_from_env, load_for_project};
    use std::ffi::OsString;
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

    #[test]
    fn home_dir_prefers_home_then_userprofile_then_home_drive_pair() {
        assert_eq!(
            home_dir_from_env(
                Some(OsString::from("/home/test")),
                Some(OsString::from("C:\\Users\\test")),
                Some(OsString::from("C:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("/home/test")
        );

        assert_eq!(
            home_dir_from_env(
                None,
                Some(OsString::from("C:\\Users\\test")),
                Some(OsString::from("D:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("C:\\Users\\test")
        );

        assert_eq!(
            home_dir_from_env(
                None,
                None,
                Some(OsString::from("D:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("D:\\Users\\fallback")
        );
    }
}
