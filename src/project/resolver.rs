use anyhow::Result;
use std::collections::{HashMap, HashSet};

use super::index::{IndexedPackage, PackageIndex};

const BUILTIN_TARGETS: &[&str] = &["FreeRTOS", "stm32cubemx"];
const DEFAULT_REPO_BASE_URL: &str = "https://github.com/HITSZ-WTRobot-Packages";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProject {
    pub direct_targets: Vec<String>,
    pub external_targets: Vec<String>,
    pub managed_packages: Vec<ResolvedPackage>,
    pub repositories: Vec<ManagedRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub pkgname: String,
    pub repo: String,
    pub path: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManagedRepository {
    pub name: String,
    pub url: String,
    pub rel_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Visited,
}

fn repo_url(repo: &str) -> String {
    format!("{DEFAULT_REPO_BASE_URL}/{repo}.git")
}

fn is_builtin_target(package: &str) -> bool {
    BUILTIN_TARGETS.contains(&package)
}

fn stable_unique(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            output.push(value.clone());
        }
    }
    output
}

fn build_package_map(index: &PackageIndex) -> Result<HashMap<String, &IndexedPackage>> {
    let mut packages = HashMap::new();
    for package in &index.packages {
        if packages.insert(package.pkgname.clone(), package).is_some() {
            anyhow::bail!("duplicate package '{}' found in index", package.pkgname);
        }
    }
    Ok(packages)
}

fn resolve_package(
    package_name: &str,
    packages: &HashMap<String, &IndexedPackage>,
    states: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
    external_targets: &mut Vec<String>,
    managed_packages: &mut Vec<ResolvedPackage>,
) -> Result<()> {
    if is_builtin_target(package_name) {
        if !external_targets.iter().any(|target| target == package_name) {
            external_targets.push(package_name.to_string());
        }
        return Ok(());
    }

    match states.get(package_name).copied() {
        Some(VisitState::Visited) => return Ok(()),
        Some(VisitState::Visiting) => {
            let mut cycle = stack.clone();
            cycle.push(package_name.to_string());
            anyhow::bail!("dependency cycle detected: {}", cycle.join(" -> "));
        }
        None => {}
    }

    let package = packages
        .get(package_name)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("package '{}' not found in index", package_name))?;

    states.insert(package_name.to_string(), VisitState::Visiting);
    stack.push(package_name.to_string());

    for dependency in &package.dependencies {
        resolve_package(
            dependency,
            packages,
            states,
            stack,
            external_targets,
            managed_packages,
        )?;
    }

    stack.pop();
    states.insert(package_name.to_string(), VisitState::Visited);
    managed_packages.push(ResolvedPackage {
        pkgname: package.pkgname.clone(),
        repo: package.repo.clone(),
        path: package.path.clone(),
        dependencies: package.dependencies.clone(),
    });
    Ok(())
}

pub fn resolve(index: &PackageIndex, requested_packages: &[String]) -> Result<ResolvedProject> {
    let direct_targets = stable_unique(requested_packages);
    let packages = build_package_map(index)?;
    let mut states = HashMap::new();
    let mut stack = Vec::new();
    let mut external_targets = Vec::new();
    let mut managed_packages = Vec::new();

    for package_name in &direct_targets {
        resolve_package(
            package_name,
            &packages,
            &mut states,
            &mut stack,
            &mut external_targets,
            &mut managed_packages,
        )?;
    }

    let mut repositories = managed_packages
        .iter()
        .map(|package| ManagedRepository {
            name: package.repo.clone(),
            url: repo_url(&package.repo),
            rel_path: format!("Modules/{}", package.repo),
        })
        .collect::<Vec<_>>();
    repositories.sort();
    repositories.dedup();

    Ok(ResolvedProject {
        direct_targets,
        external_targets,
        managed_packages,
        repositories,
    })
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use crate::project::index::{IndexedPackage, PackageIndex};

    fn sample_index() -> PackageIndex {
        PackageIndex {
            generated_at: None,
            packages: vec![
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
                    path: "Modules/MotorDrivers/motors/DJI".to_string(),
                    name: "DJI".to_string(),
                    pkgname: "MotorDrivers::DJI".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec![
                        "MotorDrivers::Core".to_string(),
                        "bsp::CANDriver".to_string(),
                    ],
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
    fn resolve_collects_transitive_packages_and_repositories() {
        let resolved = resolve(&sample_index(), &["MotorDrivers::DJI".to_string()]).unwrap();

        assert_eq!(resolved.direct_targets, vec!["MotorDrivers::DJI"]);
        assert_eq!(resolved.external_targets, vec!["stm32cubemx"]);
        assert_eq!(resolved.managed_packages.len(), 3);
        assert_eq!(resolved.repositories.len(), 2);
        assert_eq!(resolved.repositories[0].name, "BasicComponents");
        assert_eq!(resolved.repositories[1].name, "MotorDrivers");
    }

    #[test]
    fn resolve_rejects_dependency_cycles() {
        let index = PackageIndex {
            generated_at: None,
            packages: vec![
                IndexedPackage {
                    repo: "Cycle".to_string(),
                    path: "Modules/Cycle/A".to_string(),
                    name: "A".to_string(),
                    pkgname: "Cycle::A".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["Cycle::B".to_string()],
                },
                IndexedPackage {
                    repo: "Cycle".to_string(),
                    path: "Modules/Cycle/B".to_string(),
                    name: "B".to_string(),
                    pkgname: "Cycle::B".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["Cycle::A".to_string()],
                },
            ],
        };

        let error = resolve(&index, &["Cycle::A".to_string()]).unwrap_err();
        assert!(error.to_string().contains("dependency cycle detected"));
    }
}
