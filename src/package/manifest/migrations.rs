use anyhow::Result;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::{CURRENT_FORMAT_VERSION, CompileConfig, Cpkg, default_package_version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Format {
    Version(u32),
    LegacyPackageVersionField,
    LegacyNamespace,
}

trait MigrationStep {
    fn from(&self) -> Format;
    fn to(&self) -> Format;
    fn migrate(&self, content: &str) -> Result<String>;
}

#[derive(Deserialize)]
struct FormatProbe {
    format_version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentCpkg {
    format_version: u32,
    name: String,
    pkgname: String,
    #[serde(default = "default_package_version")]
    version: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    compile: CompileConfig,
    #[serde(default)]
    ignore: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPackageVersionCpkg {
    name: String,
    pkgname: String,
    version: Option<String>,
    dependencies: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyNamespaceCpkg {
    namespace: String,
    name: String,
    deps: Option<Vec<String>>,
}

struct LegacyPackageVersionMigration;

impl From<CurrentCpkg> for Cpkg {
    fn from(current: CurrentCpkg) -> Self {
        Self {
            format_version: current.format_version,
            name: current.name,
            pkgname: current.pkgname,
            version: current.version,
            dependencies: current.dependencies,
            compile: current.compile,
            ignore: current.ignore,
        }
    }
}

impl MigrationStep for LegacyPackageVersionMigration {
    fn from(&self) -> Format {
        Format::LegacyPackageVersionField
    }

    fn to(&self) -> Format {
        Format::Version(CURRENT_FORMAT_VERSION)
    }

    fn migrate(&self, content: &str) -> Result<String> {
        let legacy = toml::from_str::<LegacyPackageVersionCpkg>(content).map_err(|error| {
            anyhow::anyhow!("failed to parse legacy package-version cpkg.toml: {error}")
        })?;
        let cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: legacy.name,
            pkgname: legacy.pkgname,
            version: legacy.version.unwrap_or_else(default_package_version),
            dependencies: legacy.dependencies.unwrap_or_default(),
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        };
        toml::to_string(&cpkg).map_err(Into::into)
    }
}

struct LegacyNamespaceMigration;

impl MigrationStep for LegacyNamespaceMigration {
    fn from(&self) -> Format {
        Format::LegacyNamespace
    }

    fn to(&self) -> Format {
        Format::Version(CURRENT_FORMAT_VERSION)
    }

    fn migrate(&self, content: &str) -> Result<String> {
        let legacy = toml::from_str::<LegacyNamespaceCpkg>(content).map_err(|error| {
            anyhow::anyhow!("failed to parse legacy namespace cpkg.toml: {error}")
        })?;
        let cpkg = Cpkg {
            format_version: CURRENT_FORMAT_VERSION,
            name: legacy.name.clone(),
            pkgname: format!("{}::{}", legacy.namespace, legacy.name),
            version: default_package_version(),
            dependencies: legacy.deps.unwrap_or_default(),
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        };
        toml::to_string(&cpkg).map_err(Into::into)
    }
}

fn built_in_steps() -> Vec<Box<dyn MigrationStep>> {
    vec![
        Box::new(LegacyPackageVersionMigration),
        Box::new(LegacyNamespaceMigration),
    ]
}

fn detect_format(content: &str) -> Option<Format> {
    if let Ok(probe) = toml::from_str::<FormatProbe>(content) {
        if let Some(version) = probe.format_version {
            return Some(Format::Version(version));
        }
    }

    if toml::from_str::<LegacyPackageVersionCpkg>(content).is_ok() {
        return Some(Format::LegacyPackageVersionField);
    }

    if toml::from_str::<LegacyNamespaceCpkg>(content).is_ok() {
        return Some(Format::LegacyNamespace);
    }

    None
}

fn parse_current(content: &str) -> Result<Cpkg> {
    let cpkg = Cpkg::from(toml::from_str::<CurrentCpkg>(content)?);
    if cpkg.format_version != CURRENT_FORMAT_VERSION {
        anyhow::bail!(
            "unsupported cpkg.toml format version {}",
            cpkg.format_version
        );
    }
    Ok(cpkg)
}

pub fn load_or_migrate(content: &str) -> Result<(Cpkg, bool)> {
    if let Ok(cpkg) = parse_current(content) {
        return Ok((cpkg, false));
    }

    let steps = built_in_steps();
    let steps_by_source = steps
        .iter()
        .map(|step| (step.from(), step.as_ref()))
        .collect::<HashMap<_, _>>();

    let mut current_format =
        detect_format(content).ok_or_else(|| anyhow::anyhow!("failed to parse cpkg.toml"))?;
    let mut content = content.to_string();
    let mut visited = HashSet::new();

    loop {
        if current_format == Format::Version(CURRENT_FORMAT_VERSION) {
            return Ok((parse_current(&content)?, true));
        }

        if !visited.insert(current_format) {
            anyhow::bail!("cpkg.toml migration cycle detected at {:?}", current_format);
        }

        let step = steps_by_source.get(&current_format).ok_or_else(|| {
            anyhow::anyhow!("no cpkg.toml migration available from {:?}", current_format)
        })?;
        content = step.migrate(&content)?;
        current_format = step.to();
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, detect_format, load_or_migrate};
    use crate::package::manifest::CURRENT_FORMAT_VERSION;

    #[test]
    fn detect_format_finds_current_format_version() {
        let format = detect_format(
            r#"
format_version = 1
name = "DJI"
pkgname = "MotorDrivers::DJI"
"#,
        )
        .unwrap();

        assert_eq!(format, Format::Version(CURRENT_FORMAT_VERSION));
    }

    #[test]
    fn load_or_migrate_converts_legacy_package_version_format() {
        let (cpkg, migrated) = load_or_migrate(
            r#"
name = "DJI"
pkgname = "MotorDrivers::DJI"
version = "0.1.0"
dependencies = ["bsp::CANDriver"]
"#,
        )
        .unwrap();

        assert!(migrated);
        assert_eq!(cpkg.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(cpkg.pkgname, "MotorDrivers::DJI");
        assert_eq!(cpkg.version, "0.1.0");
        assert_eq!(cpkg.dependencies, vec!["bsp::CANDriver"]);
    }

    #[test]
    fn load_or_migrate_converts_legacy_namespace_format() {
        let (cpkg, migrated) = load_or_migrate(
            r#"
namespace = "bsp"
name = "CANDriver"
deps = ["stm32cubemx"]
"#,
        )
        .unwrap();

        assert!(migrated);
        assert_eq!(cpkg.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(cpkg.pkgname, "bsp::CANDriver");
        assert_eq!(cpkg.version, "0.1.0");
        assert_eq!(cpkg.dependencies, vec!["stm32cubemx"]);
    }

    #[test]
    fn load_or_migrate_preserves_compile_config_for_current_format() {
        let (cpkg, migrated) = load_or_migrate(
            r#"
format_version = 1
name = "DJI"
pkgname = "MotorDrivers::DJI"
version = "0.1.0"
dependencies = ["bsp::CANDriver"]

[compile]
options = ["-Ofast"]
defines = ["ARM_MATH_CM4"]
"#,
        )
        .unwrap();

        assert!(!migrated);
        assert_eq!(cpkg.compile.options, vec!["-Ofast"]);
        assert_eq!(cpkg.compile.defines, vec!["ARM_MATH_CM4"]);
    }
}
