use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::Path;

use super::manifest::{WtrProject, load, save};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DependencyEditSummary {
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
    pub(crate) unchanged: Vec<String>,
}

pub(crate) fn merge_requested_packages(
    interactive_packages: &[String],
    explicit_packages: &[String],
) -> Vec<String> {
    let mut merged = Vec::new();
    for package in interactive_packages.iter().chain(explicit_packages.iter()) {
        if !merged.contains(package) {
            merged.push(package.clone());
        }
    }
    merged
}

pub(crate) fn dependency_edit_summary(
    previous: &[String],
    next: &[String],
) -> DependencyEditSummary {
    let previous_set = previous.iter().cloned().collect::<BTreeSet<_>>();
    let next_set = next.iter().cloned().collect::<BTreeSet<_>>();

    let added = next
        .iter()
        .filter(|package| !previous_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();
    let removed = previous
        .iter()
        .filter(|package| !next_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();
    let unchanged = next
        .iter()
        .filter(|package| previous_set.contains(*package))
        .cloned()
        .collect::<Vec<_>>();

    DependencyEditSummary {
        added,
        removed,
        unchanged,
    }
}

pub(crate) fn format_package_list(packages: &[String]) -> String {
    if packages.is_empty() {
        "(none)".to_string()
    } else {
        packages.join(", ")
    }
}

pub(crate) fn is_dependency_validation_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("not found in index") || message.contains("dependency cycle detected")
    })
}

pub(crate) fn update_manifest_then<Update, FollowUp, ShouldRestore>(
    root: &Path,
    update: Update,
    follow_up: FollowUp,
    should_restore: ShouldRestore,
) -> Result<WtrProject>
where
    Update: FnOnce(&mut WtrProject),
    FollowUp: FnOnce(&WtrProject) -> Result<()>,
    ShouldRestore: Fn(&anyhow::Error) -> bool,
{
    let previous_manifest = load(root)?;
    let mut updated_manifest = previous_manifest.clone();
    update(&mut updated_manifest);
    save(root, &updated_manifest)?;
    if let Err(error) = follow_up(&updated_manifest) {
        if should_restore(&error) {
            save(root, &previous_manifest).context(
                "failed to restore previous wtrproject.toml after package validation failed",
            )?;
        }
        return Err(error);
    }
    load(root)
}

#[cfg(test)]
mod tests {
    use super::{
        dependency_edit_summary, is_dependency_validation_error, merge_requested_packages,
        update_manifest_then,
    };
    use crate::project::{ProjectInitOptions, init, load};
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
    fn merge_requested_packages_preserves_order_and_deduplicates() {
        let merged = merge_requested_packages(
            &[
                "MotorDrivers::DJI".to_string(),
                "bsp::CANDriver".to_string(),
            ],
            &[
                "bsp::CANDriver".to_string(),
                "services::Watchdog".to_string(),
            ],
        );

        assert_eq!(
            merged,
            vec!["MotorDrivers::DJI", "bsp::CANDriver", "services::Watchdog"]
        );
    }

    #[test]
    fn dependency_edit_summary_reports_added_removed_and_unchanged() {
        let summary = dependency_edit_summary(
            &[
                "MotorDrivers::DJI".to_string(),
                "bsp::CANDriver".to_string(),
                "services::Watchdog".to_string(),
            ],
            &[
                "bsp::CANDriver".to_string(),
                "services::Referee".to_string(),
                "services::Watchdog".to_string(),
            ],
        );

        assert_eq!(summary.added, vec!["services::Referee"]);
        assert_eq!(summary.removed, vec!["MotorDrivers::DJI"]);
        assert_eq!(
            summary.unchanged,
            vec!["bsp::CANDriver", "services::Watchdog"]
        );
    }

    #[test]
    fn update_manifest_then_persists_changes_before_follow_up_failure() {
        let dir = make_temp_dir("persist-before-follow-up");
        fs::write(dir.join("robot.ioc"), "").unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let error = update_manifest_then(
            &dir,
            |manifest| manifest.dependencies.packages = vec!["MotorDrivers::DJI".to_string()],
            |_| Err(anyhow::anyhow!("simulated network failure")),
            is_dependency_validation_error,
        )
        .unwrap_err();

        assert!(error.to_string().contains("simulated network failure"));

        let manifest = load(&dir).unwrap();
        assert_eq!(manifest.dependencies.packages, vec!["MotorDrivers::DJI"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn update_manifest_then_restores_previous_manifest_after_validation_failure() {
        let dir = make_temp_dir("restore-after-validation");
        fs::write(dir.join("robot.ioc"), "").unwrap();

        init(
            &dir,
            ProjectInitOptions {
                force: false,
                name: Some("robot".to_string()),
                ioc: None,
            },
        )
        .unwrap();

        let error = update_manifest_then(
            &dir,
            |manifest| manifest.dependencies.packages = vec!["invalid::Package".to_string()],
            |_| {
                Err(anyhow::anyhow!(
                    "package 'invalid::Package' not found in index"
                ))
            },
            is_dependency_validation_error,
        )
        .unwrap_err();

        assert!(error.to_string().contains("not found in index"));

        let manifest = load(&dir).unwrap();
        assert!(manifest.dependencies.packages.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dependency_validation_error_detects_not_found_messages() {
        assert!(is_dependency_validation_error(&anyhow::anyhow!(
            "package 'invalid::Package' not found in index"
        )));
        assert!(!is_dependency_validation_error(&anyhow::anyhow!(
            "simulated network failure"
        )));
    }
}
