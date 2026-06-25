pub mod generator;
pub mod manifest;
pub mod scanner;

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::info;

pub use self::generator::{CMakeGenerator, Generator};
pub use self::manifest::Cpkg;
pub use self::scanner::{DefaultFsScanner, Scanner};

use self::manifest::CURRENT_FORMAT_VERSION;

fn default_package_manifest(pkgname: &str, dependencies: Vec<String>) -> Cpkg {
    Cpkg {
        format_version: CURRENT_FORMAT_VERSION,
        name: pkgname.split("::").last().unwrap_or(pkgname).to_string(),
        pkgname: pkgname.to_string(),
        version: manifest::default_package_version(),
        dependencies,
        compile: manifest::CompileConfig::default(),
        ignore: Vec::new(),
    }
}

pub fn create(root: &Path, package_name: &str) -> Result<()> {
    let path = root.join(package_name);
    if path.exists() {
        anyhow::bail!("package folder '{}' already exists", package_name);
    }

    fs::create_dir_all(path.join("include")).context("failed to create include folder")?;
    fs::create_dir_all(path.join("src")).context("failed to create src folder")?;

    let manifest = default_package_manifest(package_name, Vec::new());
    manifest::save(&path.join("cpkg.toml"), &manifest)?;

    info!(
        "Package '{}' created with include/ and src/ folders",
        package_name
    );
    Ok(())
}

pub fn init(root: &Path, pkgname: &str, force: bool, deps: &[String]) -> Result<()> {
    let cpkg_path = root.join("cpkg.toml");
    let cmake_path = root.join("CMakeLists.txt");

    let cpkg = if cpkg_path.exists() {
        manifest::load_or_migrate_default(&cpkg_path)?
    } else {
        default_package_manifest(pkgname, deps.to_vec())
    };

    if cmake_path.exists() && !force {
        anyhow::bail!("CMakeLists.txt already exists (use -f to overwrite)");
    }

    manifest::save(&cpkg_path, &cpkg)?;
    info!("cpkg.toml generated/migrated for {}", cpkg.pkgname);

    let scanner = DefaultFsScanner::new(cpkg.ignore.clone());
    let generator = CMakeGenerator::default();
    generator
        .write_to(&cpkg, &scanner, &cmake_path)
        .context("failed to write CMakeLists.txt")?;
    Ok(())
}

pub fn generate(root: &Path) -> Result<()> {
    let cpkg_path = root.join("cpkg.toml");
    if !cpkg_path.exists() {
        anyhow::bail!("cpkg.toml not found");
    }

    let cpkg = manifest::load_or_migrate_default(&cpkg_path)?;
    let scanner = DefaultFsScanner::new(cpkg.ignore.clone());
    let generator = CMakeGenerator::default();
    generator
        .write_to(&cpkg, &scanner, &root.join("CMakeLists.txt"))
        .context("failed to write CMakeLists.txt")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::create;
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
    fn create_writes_default_version_to_manifest() {
        let dir = make_temp_dir("package-create");

        create(&dir, "MotorDrivers::DJI").unwrap();

        let manifest = fs::read_to_string(dir.join("MotorDrivers::DJI").join("cpkg.toml")).unwrap();
        assert!(manifest.contains("version = \"0.1.0\""));

        let _ = fs::remove_dir_all(dir);
    }
}
