pub mod generator;
pub mod manifest;
pub mod scanner;

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn};

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

    regenerate_from_manifest(root)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationOutcome {
    Written,
    UpToDate,
}

/// Regenerate `<package_dir>/CMakeLists.txt` from `<package_dir>/cpkg.toml`.
pub fn regenerate_from_manifest(package_dir: &Path) -> Result<GenerationOutcome> {
    let manifest_path = package_dir.join("cpkg.toml");
    let cpkg = manifest::load_for_generation(&manifest_path)
        .with_context(|| format!("failed to load '{}'", manifest_path.display()))?;
    let scanner = DefaultFsScanner::new(cpkg.ignore.clone());
    let generator = CMakeGenerator::default();
    let content = generator.generate_string(&cpkg, &scanner, package_dir);
    let target = package_dir.join("CMakeLists.txt");

    if fs::read_to_string(&target).is_ok_and(|existing| existing == content) {
        return Ok(GenerationOutcome::UpToDate);
    }

    fs::write(&target, &content)
        .with_context(|| format!("failed to write '{}'", target.display()))?;
    if is_tracked_by_git(&target) {
        warn!(
            "generated '{}' is still tracked by git; add it to .gitignore",
            target.display()
        );
    }
    Ok(GenerationOutcome::Written)
}

/// `true` when `<dir>/CMakeLists.txt` is tracked by git. Not a git repository / any git
/// failure counts as untracked so package authoring never fails on git state.
fn is_tracked_by_git(target: &Path) -> bool {
    let Some(dir) = target.parent() else {
        return false;
    };
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "CMakeLists.txt"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn generate(root: &Path) -> Result<()> {
    if !root.join("cpkg.toml").exists() {
        anyhow::bail!("cpkg.toml not found");
    }

    regenerate_from_manifest(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GenerationOutcome, create, regenerate_from_manifest};
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

    #[test]
    fn regenerate_from_manifest_writes_then_reports_up_to_date() {
        let dir = make_temp_dir("package-regenerate");
        fs::write(
            dir.join("cpkg.toml"),
            "name = \"Core\"\npkgname = \"SharedRepo::Core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(dir.join("core.c"), b"").unwrap();

        assert_eq!(
            regenerate_from_manifest(&dir).unwrap(),
            GenerationOutcome::Written
        );
        let generated = fs::read_to_string(dir.join("CMakeLists.txt")).unwrap();
        assert!(generated.contains("add_library(SharedRepoCore STATIC"));
        assert!(generated.contains("\"./core.c\""));
        assert!(generated.contains("add_library(SharedRepo::Core ALIAS SharedRepoCore)"));
        assert!(!generated.contains(&dir.to_string_lossy().to_string()));

        assert_eq!(
            regenerate_from_manifest(&dir).unwrap(),
            GenerationOutcome::UpToDate
        );

        fs::write(dir.join("extra.c"), b"").unwrap();
        assert_eq!(
            regenerate_from_manifest(&dir).unwrap(),
            GenerationOutcome::Written
        );
        let regenerated = fs::read_to_string(dir.join("CMakeLists.txt")).unwrap();
        assert!(regenerated.contains("\"./extra.c\""));

        let _ = fs::remove_dir_all(dir);
    }
}
