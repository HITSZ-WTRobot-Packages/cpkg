use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::info;

use crate::config::{self, CURRENT_FORMAT_VERSION, Cpkg};
use crate::generator::{CMakeGenerator, Generator};
use crate::scanner::DefaultFsScanner;

fn default_package_manifest(pkgname: &str, dependencies: Vec<String>) -> Cpkg {
    Cpkg {
        format_version: CURRENT_FORMAT_VERSION,
        name: pkgname.split("::").last().unwrap_or(pkgname).to_string(),
        pkgname: pkgname.to_string(),
        dependencies,
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
    config::save(&path.join("cpkg.toml"), &manifest)?;

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
        config::load_or_migrate_default(&cpkg_path)?
    } else {
        default_package_manifest(pkgname, deps.to_vec())
    };

    if cmake_path.exists() && !force {
        anyhow::bail!("CMakeLists.txt already exists (use -f to overwrite)");
    }

    config::save(&cpkg_path, &cpkg)?;
    info!("cpkg.toml generated/migrated for {}", cpkg.pkgname);

    let scanner = DefaultFsScanner;
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

    let cpkg = config::load_or_migrate_default(&cpkg_path)?;
    let scanner = DefaultFsScanner;
    let generator = CMakeGenerator::default();
    generator
        .write_to(&cpkg, &scanner, &root.join("CMakeLists.txt"))
        .context("failed to write CMakeLists.txt")?;
    Ok(())
}
