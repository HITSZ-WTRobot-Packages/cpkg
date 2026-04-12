use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use tracing::info;

use super::git::{is_registered_submodule, remove_stale_submodule_git_dir, run_git};
use crate::project::resolver::ManagedRepository;

pub(super) fn repository_names_to_remove(
    current_repositories: &[String],
    desired_repositories: &[ManagedRepository],
) -> Vec<String> {
    let desired = desired_repositories
        .iter()
        .map(|repository| repository.name.clone())
        .collect::<BTreeSet<_>>();

    current_repositories
        .iter()
        .filter(|repository| !desired.contains(*repository))
        .cloned()
        .collect()
}

pub(super) fn remove_repository(root: &Path, repository_name: &str) -> Result<()> {
    let rel_path = format!("Modules/{repository_name}");
    if !is_registered_submodule(root, &rel_path)? {
        return Ok(());
    }

    run_git(
        root,
        &["submodule", "deinit", "-f", "--", &rel_path],
        &format!("deinitializing submodule {repository_name}"),
        false,
    )?;
    run_git(
        root,
        &["rm", "-f", "--", &rel_path],
        &format!("removing submodule {repository_name}"),
        false,
    )?;
    remove_stale_submodule_git_dir(root, &rel_path)?;
    info!("removed submodule {}", repository_name);
    Ok(())
}
