use anyhow::Result;
use console::Term;

use super::integration;
use super::manifest::WtrProject;
use super::updates::{dependency_edit_summary, format_package_list};

pub(crate) fn init_guidance_lines(manifest: &WtrProject) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Next steps to integrate cpkg into your CMake project:".to_string());

    if manifest.dependencies.packages.is_empty() {
        lines.push("1. Add direct dependencies with `cpkg add <PACKAGE>`.".to_string());
        lines.push(format!(
            "2. Run `cpkg sync` to generate `{}`.",
            integration::GENERATED_CMAKE_PATH
        ));
        lines.push(
            "3. Add `include(cmake/wtr_modules.cmake)` to the root `CMakeLists.txt`.".to_string(),
        );
        lines.push(
            "4. Use `wtr_link_packages(<target>)` for plain linking, or `wtr_link_packages_public(<target>)` for `PUBLIC` linking.".to_string(),
        );
    } else {
        lines.push(format!(
            "1. Run `cpkg sync` to generate `{}`.",
            integration::GENERATED_CMAKE_PATH
        ));
        lines.push(
            "2. Add `include(cmake/wtr_modules.cmake)` to the root `CMakeLists.txt`.".to_string(),
        );
        lines.push(
            "3. Use `wtr_link_packages(<target>)` for plain linking, or `wtr_link_packages_public(<target>)` for `PUBLIC` linking.".to_string(),
        );
    }

    lines
}

pub fn write_init_integration_guidance(manifest: &WtrProject) -> Result<()> {
    let term = Term::stderr();
    for line in init_guidance_lines(manifest) {
        term.write_line(&line)?;
    }
    Ok(())
}

pub(crate) fn add_sync_deferred_notice_lines(
    repository_name: &str,
    sync_command: &str,
) -> Vec<String> {
    vec![
        format!(
            "Updated `wtrproject.toml` only; `{}` and `Modules/` were left unchanged.",
            integration::GENERATED_CMAKE_PATH
        ),
        format!(
            "Applying this add requires fetching repository '{}'.",
            repository_name
        ),
        format!("Run `{sync_command}` online to apply the change."),
    ]
}

pub(crate) fn write_add_sync_deferred_notice(
    repository_name: &str,
    sync_command: &str,
) -> Result<()> {
    let term = Term::stderr();
    for line in add_sync_deferred_notice_lines(repository_name, sync_command) {
        term.write_line(&line)?;
    }
    Ok(())
}

pub(crate) fn write_add_interactive_summary(previous: &[String], next: &[String]) -> Result<()> {
    let summary = dependency_edit_summary(previous, next);
    let term = Term::stderr();

    term.write_line(&format!(
        "Direct dependency changes: {} added, {} removed, {} unchanged.",
        summary.added.len(),
        summary.removed.len(),
        summary.unchanged.len()
    ))?;
    if !summary.added.is_empty() {
        term.write_line(&format!("Added: {}", format_package_list(&summary.added)))?;
    }
    if !summary.removed.is_empty() {
        term.write_line(&format!(
            "Removed: {}",
            format_package_list(&summary.removed)
        ))?;
    }
    term.write_line(&format!("Current: {}", format_package_list(next)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{add_sync_deferred_notice_lines, init_guidance_lines};
    use crate::project::manifest::CURRENT_FORMAT_VERSION;
    use crate::project::{DependencySection, IndexSection, ProjectSection, WtrProject};

    #[test]
    fn init_guidance_mentions_add_before_sync_when_dependencies_are_empty() {
        let lines = init_guidance_lines(&WtrProject {
            format_version: CURRENT_FORMAT_VERSION,
            project: ProjectSection {
                name: "demo".to_string(),
                ioc_file: "demo.ioc".to_string(),
            },
            dependencies: DependencySection::default(),
            index: IndexSection::default(),
        });

        assert!(lines.iter().any(|line| line.contains("cpkg add <PACKAGE>")));
        assert!(lines.iter().any(|line| {
            line.contains("wtr_link_packages(<target>)")
                && line.contains("wtr_link_packages_public(<target>)")
        }));
    }

    #[test]
    fn init_guidance_skips_add_hint_when_dependencies_are_present() {
        let lines = init_guidance_lines(&WtrProject {
            format_version: CURRENT_FORMAT_VERSION,
            project: ProjectSection {
                name: "demo".to_string(),
                ioc_file: "demo.ioc".to_string(),
            },
            dependencies: DependencySection {
                packages: vec!["MotorDrivers::DJI".to_string()],
            },
            index: IndexSection::default(),
        });

        assert!(!lines.iter().any(|line| line.contains("cpkg add <PACKAGE>")));
        assert!(lines.iter().any(|line| line.contains("cpkg sync")));
    }

    #[test]
    fn add_sync_deferred_notice_mentions_online_sync() {
        let lines = add_sync_deferred_notice_lines(
            "BasicComponents",
            "cpkg sync --submodule-protocol https",
        );

        assert!(lines.iter().any(|line| line.contains("wtrproject.toml")));
        assert!(lines.iter().any(|line| line.contains("BasicComponents")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("cpkg sync --submodule-protocol https"))
        );
    }
}
