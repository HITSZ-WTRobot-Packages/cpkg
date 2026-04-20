use anyhow::Result;
use console::{Term, colors_enabled, style};
use std::path::Path;
use tracing::debug;

use super::index::{self, PackageIndex};
use super::interactive::{PackageChoice, group_packages_by_repository};
use super::manifest::{
    self, CURRENT_FORMAT_VERSION, DependencySection, IndexSection, OrgSection, ProjectSection,
    WtrProject,
};

fn default_listing_manifest() -> WtrProject {
    WtrProject {
        format_version: CURRENT_FORMAT_VERSION,
        project: ProjectSection {
            name: "available_packages".to_string(),
            ioc_file: "available_packages.ioc".to_string(),
        },
        dependencies: DependencySection::default(),
        index: IndexSection::default(),
        org: OrgSection::default(),
    }
}

fn load_index_for_listing(root: &Path, offline: bool) -> Result<PackageIndex> {
    let manifest = if manifest::manifest_path(root).exists() {
        manifest::load(root)?
    } else {
        default_listing_manifest()
    };

    if offline {
        index::load_for_project_without_refresh(root, &manifest)
    } else {
        index::load_for_project(root, &manifest)
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn branch(is_last: bool) -> &'static str {
    if is_last { "└──" } else { "├──" }
}

fn child_prefix(parent_is_last: bool) -> &'static str {
    if parent_is_last { "    " } else { "│   " }
}

fn package_details(package: &PackageChoice) -> Vec<String> {
    let mut details = Vec::new();
    if !package.version.is_empty() {
        details.push(format!("v{}", package.version));
    }
    details.push(package.path.clone());
    if !package.dependencies.is_empty() {
        details.push(format!("deps: {}", package.dependencies.join(", ")));
    }
    details
}

fn maybe_style(
    value: impl std::fmt::Display,
    color_enabled: bool,
) -> console::StyledObject<String> {
    style(value.to_string()).force_styling(color_enabled)
}

fn header_label(index: &PackageIndex, repository_count: usize, color_enabled: bool) -> String {
    let title = maybe_style("packages", color_enabled).cyan().bold();
    let count = maybe_style(
        format!(
            "({} total, {})",
            index.packages.len(),
            count_label(repository_count, "repository", "repositories")
        ),
        color_enabled,
    )
    .dim();
    format!("{title} {count}")
}

fn tree_marker(value: &str, color_enabled: bool) -> String {
    maybe_style(value, color_enabled).dim().to_string()
}

fn repository_label(repo: &str, color_enabled: bool) -> String {
    maybe_style(repo, color_enabled).blue().bold().to_string()
}

fn package_label_with_color(package: &PackageChoice, color_enabled: bool) -> String {
    let pkgname = maybe_style(&package.pkgname, color_enabled)
        .green()
        .bold()
        .to_string();
    let details = package_details(package);
    if details.is_empty() {
        pkgname
    } else {
        let details = maybe_style(format!("[{}]", details.join(" | ")), color_enabled)
            .dim()
            .to_string();
        format!("{pkgname} {details}")
    }
}

pub(crate) fn render_package_tree_lines_with_color(
    index: &PackageIndex,
    color_enabled: bool,
) -> Vec<String> {
    let groups = group_packages_by_repository(index);
    let mut lines = Vec::new();
    lines.push(header_label(index, groups.len(), color_enabled));

    for (repo_index, group) in groups.iter().enumerate() {
        let repo_is_last = repo_index + 1 == groups.len();
        lines.push(format!(
            "{} {}",
            tree_marker(branch(repo_is_last), color_enabled),
            repository_label(&group.repo, color_enabled)
        ));

        let package_prefix = child_prefix(repo_is_last);
        for (package_index, package) in group.packages.iter().enumerate() {
            let package_is_last = package_index + 1 == group.packages.len();
            lines.push(format!(
                "{}{} {}",
                tree_marker(package_prefix, color_enabled),
                tree_marker(branch(package_is_last), color_enabled),
                package_label_with_color(package, color_enabled)
            ));
        }
    }

    lines
}

pub(crate) fn render_package_tree_lines(index: &PackageIndex) -> Vec<String> {
    render_package_tree_lines_with_color(index, false)
}

pub fn list_available_packages(root: &Path, offline: bool) -> Result<()> {
    let index = load_index_for_listing(root, offline)?;
    debug!(
        offline,
        indexed_package_count = index.packages.len(),
        root = %root.display(),
        "listing available packages"
    );

    let term = Term::stdout();
    let lines = if term.is_term() && colors_enabled() {
        render_package_tree_lines_with_color(&index, true)
    } else {
        render_package_tree_lines(&index)
    };
    for line in lines {
        term.write_line(&line)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        load_index_for_listing, render_package_tree_lines, render_package_tree_lines_with_color,
    };
    use crate::project::index::{IndexedPackage, PackageIndex};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cpkg-listing-{prefix}-{}-{}",
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
    fn render_package_tree_lines_groups_packages_by_repository() {
        let lines = render_package_tree_lines(&sample_index());

        assert_eq!(
            lines,
            vec![
                "packages (3 total, 2 repositories)".to_string(),
                "├── BasicComponents".to_string(),
                "│   └── bsp::CANDriver [v0.1.0 | bsp/can_driver | deps: stm32cubemx]".to_string(),
                "└── MotorDrivers".to_string(),
                "    ├── MotorDrivers::Core [v0.1.0 | core]".to_string(),
                "    └── MotorDrivers::DJI [v0.1.0 | motors/DJI | deps: bsp::CANDriver]"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn render_package_tree_lines_with_color_styles_tree_output() {
        let lines = render_package_tree_lines_with_color(&sample_index(), true);

        assert!(lines[0].contains("\u{1b}["));
        assert!(lines[0].contains("packages"));
        assert!(lines[1].contains("\u{1b}["));
        assert!(lines[1].contains("BasicComponents"));
        assert!(lines[2].contains("\u{1b}["));
        assert!(lines[2].contains("bsp::CANDriver"));
    }

    #[test]
    fn load_index_for_listing_supports_project_local_index_without_manifest() {
        let dir = make_temp_dir("project-local-index");
        fs::write(
            dir.join("cpkg_index.json"),
            r#"{
  "packages": [
    {
      "repo": "DemoRepo",
      "path": "Modules/DemoRepo/demo/pkg",
      "name": "Pkg",
      "pkgname": "Demo::Pkg",
      "version": "1.0.0",
      "dependencies": []
    }
  ]
}"#,
        )
        .unwrap();

        let index = load_index_for_listing(&dir, true).unwrap();

        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].pkgname, "Demo::Pkg");

        let _ = fs::remove_dir_all(dir);
    }
}
