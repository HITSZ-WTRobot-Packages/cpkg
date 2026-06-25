mod migrations;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Current canonical `cpkg.toml` format version.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// Per-package compile options and preprocessor definitions.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CompileConfig {
    /// Compiler flags (e.g. `-Ofast`, `-ffast-math`).
    /// Order is preserved — compiler flag order can be semantically significant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Preprocessor definitions (e.g. `ARM_MATH_CM4`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defines: Vec<String>,
}

impl CompileConfig {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty() && self.defines.is_empty()
    }
}

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
    #[serde(default, skip_serializing_if = "CompileConfig::is_empty")]
    pub compile: CompileConfig,
    /// Glob patterns for files and directories to exclude from scanning.
    /// Gitignore-style: `*.o`, `build/`, `**/test/**`, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
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

fn normalize_compile(compile: &mut CompileConfig) {
    compile.defines.sort();
    compile.defines.dedup();
}

fn normalize_manifest(cpkg: &mut Cpkg) {
    if cpkg.version.trim().is_empty() {
        cpkg.version = default_package_version();
    }
    normalize_dependencies(&mut cpkg.dependencies);
    normalize_compile(&mut cpkg.compile);
    cpkg.ignore.sort();
    cpkg.ignore.dedup();
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
    use super::{Cpkg, CompileConfig, CURRENT_FORMAT_VERSION, load_or_migrate_default};
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

    #[test]
    fn load_parses_compile_section() {
        let dir = make_temp_dir("config-compile");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
format_version = 1
name = "DJI"
pkgname = "MotorDrivers::DJI"
version = "0.1.0"
dependencies = ["bsp::CANDriver"]

[compile]
options = ["-Ofast", "-ffast-math"]
defines = ["ARM_MATH_CM4"]
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();

        assert_eq!(cpkg.compile.options, vec!["-Ofast", "-ffast-math"]);
        assert_eq!(cpkg.compile.defines, vec!["ARM_MATH_CM4"]);
        assert!(!cpkg.compile.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_missing_compile_section_defaults_to_empty() {
        let dir = make_temp_dir("config-no-compile");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
format_version = 1
name = "DJI"
pkgname = "MotorDrivers::DJI"
dependencies = []
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();
        assert!(cpkg.compile.is_empty());
        assert!(cpkg.compile.options.is_empty());
        assert!(cpkg.compile.defines.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_skips_empty_compile_section() {
        let dir = make_temp_dir("config-save-skip");
        let path = dir.join("cpkg.toml");
        let cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: "test".to_string(),
            pkgname: "Test::Lib".to_string(),
            version: "0.1.0".to_string(),
            dependencies: vec![],
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        };
        super::save(&path, &cpkg).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("[compile]"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_emits_compile_section_when_populated() {
        let dir = make_temp_dir("config-save-emit");
        let path = dir.join("cpkg.toml");
        let cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: "test".to_string(),
            pkgname: "Test::Lib".to_string(),
            version: "0.1.0".to_string(),
            dependencies: vec![],
            compile: CompileConfig {
                options: vec!["-Ofast".to_string()],
                defines: vec![],
            },
            ignore: Vec::new(),
        };
        super::save(&path, &cpkg).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[compile]"));
        assert!(content.contains("-Ofast"));
        assert!(!content.contains("defines"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_compile_sorts_and_dedup_defines() {
        let dir = make_temp_dir("config-norm-defines");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
format_version = 1
name = "test"
pkgname = "Test::Lib"

[compile]
defines = ["B", "A", "B"]
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();
        // defines should be sorted and deduplicated
        assert_eq!(cpkg.compile.defines, vec!["A", "B"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_parses_ignore_patterns() {
        let dir = make_temp_dir("config-ignore");
        let path = dir.join("cpkg.toml");
        fs::write(
            &path,
            r#"
format_version = 1
name = "test"
pkgname = "Test::Lib"
ignore = ["build/**", "**/*.o"]
"#,
        )
        .unwrap();

        let cpkg = load_or_migrate_default(&path).unwrap();

        assert_eq!(cpkg.ignore, vec!["**/*.o", "build/**"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_skips_empty_ignore() {
        let dir = make_temp_dir("config-ignore-save");
        let path = dir.join("cpkg.toml");
        let cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: "test".to_string(),
            pkgname: "Test::Lib".to_string(),
            version: "0.1.0".to_string(),
            dependencies: vec![],
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        };
        super::save(&path, &cpkg).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("ignore"));

        let _ = fs::remove_dir_all(dir);
    }
}
