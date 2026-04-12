use std::collections::BTreeSet;

use crate::project::index::{IndexedPackage, PackageIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPackageGroup {
    pub repo: String,
    pub packages: Vec<PackageChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChoice {
    pub pkgname: String,
    pub path: String,
    pub version: String,
    pub dependencies: Vec<String>,
}

fn package_relative_path(package: &IndexedPackage) -> String {
    let repo_prefix = format!("Modules/{}/", package.repo);
    package
        .path
        .strip_prefix(&repo_prefix)
        .unwrap_or(&package.path)
        .to_string()
}

pub fn group_packages_by_repository(index: &PackageIndex) -> Vec<RepositoryPackageGroup> {
    let repos = index
        .packages
        .iter()
        .map(|package| package.repo.clone())
        .collect::<BTreeSet<_>>();

    repos
        .into_iter()
        .map(|repo| {
            let mut packages = index
                .packages
                .iter()
                .filter(|package| package.repo == repo)
                .map(|package| PackageChoice {
                    pkgname: package.pkgname.clone(),
                    path: package_relative_path(package),
                    version: package.version.clone(),
                    dependencies: package.dependencies.clone(),
                })
                .collect::<Vec<_>>();
            packages.sort_by(|left, right| left.pkgname.cmp(&right.pkgname));
            RepositoryPackageGroup { repo, packages }
        })
        .collect()
}

pub(super) fn selected_package_indices(
    groups: &[RepositoryPackageGroup],
    initially_selected_packages: &[String],
) -> BTreeSet<(usize, usize)> {
    let selected_names = initially_selected_packages
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut selected_indices = BTreeSet::new();
    for (repo_index, group) in groups.iter().enumerate() {
        for (package_index, package) in group.packages.iter().enumerate() {
            if selected_names.contains(&package.pkgname) {
                selected_indices.insert((repo_index, package_index));
            }
        }
    }
    selected_indices
}

#[cfg(test)]
mod tests {
    use super::{group_packages_by_repository, selected_package_indices};
    use crate::project::index::{IndexedPackage, PackageIndex};

    fn sample_index() -> PackageIndex {
        PackageIndex {
            generated_at: None,
            packages: vec![
                IndexedPackage {
                    repo: "MotorDrivers".to_string(),
                    path: "Modules/MotorDrivers/motors/DJI".to_string(),
                    name: "DJI".to_string(),
                    pkgname: "MotorDrivers::DJI".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["bsp::CANDriver".to_string()],
                },
                IndexedPackage {
                    repo: "BasicComponents".to_string(),
                    path: "Modules/BasicComponents/bsp/can_driver".to_string(),
                    name: "CANDriver".to_string(),
                    pkgname: "bsp::CANDriver".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["stm32cubemx".to_string()],
                },
                IndexedPackage {
                    repo: "MotorDrivers".to_string(),
                    path: "Modules/MotorDrivers/core".to_string(),
                    name: "Core".to_string(),
                    pkgname: "MotorDrivers::Core".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn group_packages_by_repository_sorts_repositories_and_packages() {
        let groups = group_packages_by_repository(&sample_index());

        assert_eq!(groups[0].repo, "BasicComponents");
        assert_eq!(groups[0].packages[0].pkgname, "bsp::CANDriver");
        assert_eq!(groups[0].packages[0].path, "bsp/can_driver");
        assert_eq!(groups[0].packages[0].version, "0.1.0");
        assert_eq!(groups[1].repo, "MotorDrivers");
        assert_eq!(groups[1].packages[0].pkgname, "MotorDrivers::Core");
        assert_eq!(groups[1].packages[1].pkgname, "MotorDrivers::DJI");
    }

    #[test]
    fn selected_package_indices_marks_initially_selected_dependencies() {
        let groups = group_packages_by_repository(&sample_index());

        let selected = selected_package_indices(
            &groups,
            &[
                "MotorDrivers::DJI".to_string(),
                "missing::Package".to_string(),
            ],
        );

        assert!(selected.contains(&(1, 1)));
        assert_eq!(selected.len(), 1);
    }
}
