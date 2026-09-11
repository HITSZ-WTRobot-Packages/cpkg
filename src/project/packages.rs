use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use super::resolver::ResolvedPackage;
use crate::package::{self, GenerationOutcome};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PackageGenerationSummary {
    pub(crate) written: usize,
    pub(crate) up_to_date: usize,
    pub(crate) skipped: usize,
}

fn package_dir(root: &Path, package: &ResolvedPackage) -> PathBuf {
    package
        .path
        .split('/')
        .fold(root.to_path_buf(), |dir, segment| dir.join(segment))
}

pub(crate) fn regenerate_package_cmake_files(
    root: &Path,
    packages: &[ResolvedPackage],
) -> Result<PackageGenerationSummary> {
    let mut summary = PackageGenerationSummary::default();
    for package in packages {
        let dir = package_dir(root, package);
        if !dir.join("cpkg.toml").exists() {
            warn!(
                package = %package.pkgname,
                path = %package.path,
                "managed package has no cpkg.toml; skipping CMakeLists.txt generation"
            );
            summary.skipped += 1;
            continue;
        }

        match package::regenerate_from_manifest(&dir).with_context(|| {
            format!(
                "failed to generate CMakeLists.txt for package '{}' at '{}'",
                package.pkgname, package.path
            )
        })? {
            GenerationOutcome::Written => summary.written += 1,
            GenerationOutcome::UpToDate => summary.up_to_date += 1,
        }
        debug!(
            package = %package.pkgname,
            path = %package.path,
            "regenerated package CMakeLists.txt"
        );
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::regenerate_package_cmake_files;
    use crate::project::resolver::ResolvedPackage;
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

    fn package(pkgname: &str, path: &str) -> ResolvedPackage {
        ResolvedPackage {
            pkgname: pkgname.to_string(),
            repo: "SharedRepo".to_string(),
            path: path.to_string(),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn regenerates_package_cmake_files_and_skips_missing_manifests() {
        let root = make_temp_dir("project-packages");
        let core = root.join("Modules").join("SharedRepo").join("core");
        let feature_a = root.join("Modules").join("SharedRepo").join("feature_a");
        fs::create_dir_all(&core).unwrap();
        fs::create_dir_all(&feature_a).unwrap();
        fs::write(
            core.join("cpkg.toml"),
            "name = \"Core\"\npkgname = \"SharedRepo::Core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(core.join("core.c"), b"").unwrap();

        let summary = regenerate_package_cmake_files(
            &root,
            &[
                package("SharedRepo::Core", "Modules/SharedRepo/core"),
                package("SharedRepo::FeatureA", "Modules/SharedRepo/feature_a"),
            ],
        )
        .unwrap();

        assert_eq!(summary.written, 1);
        assert_eq!(summary.up_to_date, 0);
        assert_eq!(summary.skipped, 1);

        let generated = fs::read_to_string(core.join("CMakeLists.txt")).unwrap();
        assert!(generated.contains("add_library(SharedRepoCore STATIC"));
        assert!(!generated.contains(&root.to_string_lossy().to_string()));
        assert!(!feature_a.join("CMakeLists.txt").exists());

        let _ = fs::remove_dir_all(root);
    }
}
