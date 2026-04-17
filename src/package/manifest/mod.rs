mod migrations;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Current canonical `cpkg.toml` format version.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Cpkg {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    pub name: String,
    pub pkgname: String,
    #[serde(default = "default_package_version")]
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

fn default_format_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

pub(crate) fn default_package_version() -> String {
    "0.1.0".to_string()
}

fn normalize_dependencies(dependencies: &mut Vec<String>) {
    dependencies.sort();
    dependencies.dedup();
}

fn normalize_manifest(cpkg: &mut Cpkg) {
    if cpkg.version.trim().is_empty() {
        cpkg.version = default_package_version();
    }
    normalize_dependencies(&mut cpkg.dependencies);
}

pub fn save(path: &Path, cpkg: &Cpkg) -> Result<()> {
    let mut normalized = cpkg.clone();
    normalize_manifest(&mut normalized);
    let content = toml::to_string(&normalized).context("failed to serialize cpkg.toml")?;
    fs::write(path, content).context("failed to write cpkg.toml")?;
    Ok(())
}

pub fn load_or_migrate_default(path: &Path) -> Result<Cpkg> {
    let content = fs::read_to_string(path).context("failed to read cpkg.toml")?;
    let (mut cpkg, migrated) = migrations::load_or_migrate(&content)?;
    normalize_manifest(&mut cpkg);
    if migrated {
        save(path, &cpkg)?;
    }
    Ok(cpkg)
}

#[cfg(test)]
mod tests {
    use super::{CURRENT_FORMAT_VERSION, load_or_migrate_default};
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
    fn load_or_migrate_converts_legacy_version_field() {
        let dir = make_temp_dir("config-legacy");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
name = "DJI"
pkgname = "MotorDrivers::DJI"
version = "0.1.0"
dependencies = ["bsp::CANDriver", "services::Watchdog"]
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();

        assert_eq!(cpkg.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(cpkg.pkgname, "MotorDrivers::DJI");
        assert_eq!(cpkg.version, "0.1.0");
        assert_eq!(cpkg.dependencies.len(), 2);

        let migrated = fs::read_to_string(&path).unwrap();
        assert!(migrated.contains("format_version = 1"));
        assert!(migrated.contains("version = \"0.1.0\""));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_or_migrate_converts_namespace_format() {
        let dir = make_temp_dir("config-old");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
namespace = "bsp"
name = "CANDriver"
deps = ["stm32cubemx"]
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();

        assert_eq!(cpkg.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(cpkg.pkgname, "bsp::CANDriver");
        assert_eq!(cpkg.version, "0.1.0");
        assert_eq!(cpkg.dependencies, vec!["stm32cubemx"]);

        let _ = fs::remove_dir_all(dir);
    }
}
