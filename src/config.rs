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
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCpkg {
    name: String,
    pkgname: String,
    version: Option<String>,
    dependencies: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OldCpkg {
    namespace: String,
    name: String,
    deps: Option<Vec<String>>,
}

fn default_format_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

fn normalize_dependencies(dependencies: &mut Vec<String>) {
    dependencies.sort();
    dependencies.dedup();
}

fn normalize_manifest(cpkg: &mut Cpkg) {
    normalize_dependencies(&mut cpkg.dependencies);
}

pub fn save(path: &Path, cpkg: &Cpkg) -> Result<()> {
    let mut normalized = cpkg.clone();
    normalize_manifest(&mut normalized);
    let content = toml::to_string(&normalized).context("failed to serialize cpkg.toml")?;
    fs::write(path, content).context("failed to write cpkg.toml")?;
    Ok(())
}

fn migrate_legacy(content: &str) -> Option<Cpkg> {
    toml::from_str::<LegacyCpkg>(content).ok().map(|legacy| {
        let mut cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: legacy.name,
            pkgname: legacy.pkgname,
            dependencies: legacy.dependencies.unwrap_or_default(),
        };
        let _ = legacy.version;
        normalize_manifest(&mut cpkg);
        cpkg
    })
}

fn migrate_old(content: &str) -> Option<Cpkg> {
    toml::from_str::<OldCpkg>(content).ok().map(|old| {
        let mut cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: old.name.clone(),
            pkgname: format!("{}::{}", old.namespace, old.name),
            dependencies: old.deps.unwrap_or_default(),
        };
        normalize_manifest(&mut cpkg);
        cpkg
    })
}

pub fn load_or_migrate_default(path: &Path) -> Result<Cpkg> {
    let content = fs::read_to_string(path).context("failed to read cpkg.toml")?;

    if let Ok(mut cpkg) = toml::from_str::<Cpkg>(&content) {
        if cpkg.format_version != CURRENT_FORMAT_VERSION {
            anyhow::bail!(
                "unsupported cpkg.toml format version {}",
                cpkg.format_version
            );
        }
        normalize_manifest(&mut cpkg);
        return Ok(cpkg);
    }

    if let Some(cpkg) = migrate_legacy(&content) {
        save(path, &cpkg)?;
        return Ok(cpkg);
    }

    if let Some(cpkg) = migrate_old(&content) {
        save(path, &cpkg)?;
        return Ok(cpkg);
    }

    anyhow::bail!("failed to parse cpkg.toml")
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
        assert_eq!(cpkg.dependencies.len(), 2);

        let migrated = fs::read_to_string(&path).unwrap();
        assert!(migrated.contains("format_version = 1"));
        assert!(
            !migrated
                .lines()
                .any(|line| line.trim_start().starts_with("version = "))
        );

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
        assert_eq!(cpkg.dependencies, vec!["stm32cubemx"]);

        let _ = fs::remove_dir_all(dir);
    }
}
