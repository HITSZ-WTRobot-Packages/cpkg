use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::io::{BufRead, Write};

use super::index::{IndexedPackage, PackageIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPackageGroup {
    pub repo: String,
    pub packages: Vec<PackageChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChoice {
    pub pkgname: String,
    pub path: String,
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
                    dependencies: package.dependencies.clone(),
                })
                .collect::<Vec<_>>();
            packages.sort_by(|left, right| left.pkgname.cmp(&right.pkgname));
            RepositoryPackageGroup { repo, packages }
        })
        .collect()
}

fn parse_number(value: &str, max: usize) -> Result<usize> {
    let number = value
        .parse::<usize>()
        .with_context(|| format!("invalid selection '{}'", value))?;
    if number == 0 || number > max {
        anyhow::bail!("selection '{}' is out of range 1..={}", value, max);
    }
    Ok(number - 1)
}

pub fn parse_selection(input: &str, max: usize) -> Result<Vec<usize>> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("q")
        || trimmed.eq_ignore_ascii_case("quit")
    {
        return Ok(Vec::new());
    }
    if trimmed.eq_ignore_ascii_case("all") || trimmed == "*" {
        return Ok((0..max).collect());
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for token in trimmed
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
    {
        if let Some((start, end)) = token.split_once('-') {
            let start = parse_number(start.trim(), max)?;
            let end = parse_number(end.trim(), max)?;
            if start > end {
                anyhow::bail!("invalid descending selection range '{}'", token);
            }
            for value in start..=end {
                if seen.insert(value) {
                    selected.push(value);
                }
            }
        } else {
            let value = parse_number(token, max)?;
            if seen.insert(value) {
                selected.push(value);
            }
        }
    }

    Ok(selected)
}

fn prompt_selection<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    max: usize,
) -> Result<Vec<usize>> {
    loop {
        write!(output, "{prompt}")?;
        output.flush()?;

        let mut line = String::new();
        let bytes_read = input.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(Vec::new());
        }

        match parse_selection(&line, max) {
            Ok(selection) => return Ok(selection),
            Err(error) => {
                writeln!(output, "Invalid selection: {error}")?;
            }
        }
    }
}

pub fn select_dependencies<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    index: &PackageIndex,
) -> Result<Vec<String>> {
    let groups = group_packages_by_repository(index);
    if groups.is_empty() {
        writeln!(output, "No packages found in the package index.")?;
        return Ok(Vec::new());
    }

    writeln!(output, "Select driver packages to add to wtrproject.toml.")?;
    writeln!(
        output,
        "Packages are grouped by repository to show which small driver libraries come from each larger repo."
    )?;
    writeln!(
        output,
        "Enter numbers like `1,3,5-7`, `all` to select all, or press Enter to skip.\n"
    )?;

    writeln!(output, "Repositories:")?;
    for (index, group) in groups.iter().enumerate() {
        writeln!(
            output,
            "  {}) {} ({} packages)",
            index + 1,
            group.repo,
            group.packages.len()
        )?;
    }

    let selected_repositories = prompt_selection(
        input,
        output,
        "\nSelect repositories to browse: ",
        groups.len(),
    )?;

    let mut selected_packages = Vec::new();
    let mut seen_packages = BTreeSet::new();
    for repository_index in selected_repositories {
        let group = &groups[repository_index];
        writeln!(output, "\n{} packages:", group.repo)?;
        for (package_index, package) in group.packages.iter().enumerate() {
            let dependency_hint = if package.dependencies.is_empty() {
                String::new()
            } else {
                format!(" deps: {}", package.dependencies.join(", "))
            };
            writeln!(
                output,
                "  {}) {}  ({}){}",
                package_index + 1,
                package.pkgname,
                package.path,
                dependency_hint
            )?;
        }

        let prompt = format!("Select packages from {}: ", group.repo);
        for package_index in prompt_selection(input, output, &prompt, group.packages.len())? {
            let pkgname = group.packages[package_index].pkgname.clone();
            if seen_packages.insert(pkgname.clone()) {
                selected_packages.push(pkgname);
            }
        }
    }

    if selected_packages.is_empty() {
        writeln!(output, "\nNo packages selected.")?;
    } else {
        writeln!(
            output,
            "\nSelected {} package(s): {}",
            selected_packages.len(),
            selected_packages.join(", ")
        )?;
    }

    Ok(selected_packages)
}

#[cfg(test)]
mod tests {
    use super::{group_packages_by_repository, parse_selection, select_dependencies};
    use crate::project::index::{IndexedPackage, PackageIndex};
    use std::io::Cursor;

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
    fn parse_selection_accepts_ranges_and_deduplicates() {
        assert_eq!(parse_selection("1, 3-4 3", 5).unwrap(), vec![0, 2, 3]);
    }

    #[test]
    fn parse_selection_accepts_all_and_empty() {
        assert_eq!(parse_selection("all", 3).unwrap(), vec![0, 1, 2]);
        assert!(parse_selection("", 3).unwrap().is_empty());
        assert!(parse_selection("q", 3).unwrap().is_empty());
    }

    #[test]
    fn group_packages_by_repository_sorts_repositories_and_packages() {
        let groups = group_packages_by_repository(&sample_index());

        assert_eq!(groups[0].repo, "BasicComponents");
        assert_eq!(groups[0].packages[0].pkgname, "bsp::CANDriver");
        assert_eq!(groups[0].packages[0].path, "bsp/can_driver");
        assert_eq!(groups[1].repo, "MotorDrivers");
        assert_eq!(groups[1].packages[0].pkgname, "MotorDrivers::Core");
        assert_eq!(groups[1].packages[1].pkgname, "MotorDrivers::DJI");
    }

    #[test]
    fn select_dependencies_walks_repository_then_package_selection() {
        let mut input = Cursor::new("2\n1-2\n");
        let mut output = Vec::new();

        let selected = select_dependencies(&mut input, &mut output, &sample_index()).unwrap();

        assert_eq!(selected, vec!["MotorDrivers::Core", "MotorDrivers::DJI"]);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("MotorDrivers (2 packages)"));
        assert!(output.contains("MotorDrivers packages:"));
    }
}
