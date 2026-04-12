use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use tracing::info;

use super::git::{
    has_git_index_entry, is_registered_submodule, remove_stale_submodule_git_dir,
    remove_submodule_registration, run_git,
};
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
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|repository| !desired.contains(repository))
        .collect()
}

pub(super) fn remove_repository(root: &Path, repository_name: &str) -> Result<()> {
    let rel_path = format!("Modules/{repository_name}");
    let registered = is_registered_submodule(root, &rel_path)?;
    let tracked = has_git_index_entry(root, &rel_path)?;

    if tracked {
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
    }

    if registered {
        remove_submodule_registration(root, &rel_path)?;
    }

    remove_stale_submodule_git_dir(root, &rel_path)?;
    info!("removed submodule {}", repository_name);
    Ok(())
}
