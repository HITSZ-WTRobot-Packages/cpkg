use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
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
            &["update-index", "--force-remove", "--", &rel_path],
            &format!("removing git index entry for submodule {repository_name}"),
            false,
        )?;

        let abs_path = root.join(&rel_path);
        if abs_path.exists() {
            fs::remove_dir_all(&abs_path).with_context(|| {
                format!(
                    "failed to remove submodule working tree '{}'",
                    abs_path.display()
                )
            })?;
        }
    }

    if registered {
        remove_submodule_registration(root, &rel_path)?;
    }

    remove_stale_submodule_git_dir(root, &rel_path)?;
    info!("removed submodule {}", repository_name);
    Ok(())
}
