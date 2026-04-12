use anyhow::Result;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tracing::info;

use super::git::{
    align_main_to_origin, ensure_main_checked_out, is_initialized_submodule,
    is_registered_submodule, remove_stale_submodule_git_dir, run_git, run_git_concurrent,
};
use crate::project::network::{ConcurrentLogState, log_concurrent_event};
use crate::project::resolver::ManagedRepository;

pub(super) const NETWORK_SYNC_MAX_ATTEMPTS: usize = 2;
const NETWORK_SYNC_RETRY_DELAY_MS: u64 = 300;

#[derive(Debug, Clone)]
pub(super) enum PendingNetworkSync {
    Initialize {
        root: PathBuf,
        repository: ManagedRepository,
    },
    Pull {
        path: PathBuf,
        repository_name: String,
    },
}

impl PendingNetworkSync {
    fn repository_name(&self) -> &str {
        match self {
            Self::Initialize { repository, .. } => &repository.name,
            Self::Pull {
                repository_name, ..
            } => repository_name,
        }
    }

    fn execute(self) -> Result<()> {
        match self {
            Self::Initialize { root, repository } => {
                let rel_path = repository.rel_path.as_str();
                let abs_path = root.join(rel_path);
                run_network_sync_with_retry(&repository.name, "initializing submodule", || {
                    run_git_concurrent(
                        &root,
                        &["submodule", "update", "--init", "--remote", "--", rel_path],
                        &format!("initializing submodule {}", repository.name),
                        &repository.name,
                    )
                    .map(|_| ())
                })?;
                align_main_to_origin(&abs_path, &repository.name)?;
                info!("synchronized submodule {}", repository.name);
                Ok(())
            }
            Self::Pull {
                path,
                repository_name,
            } => {
                run_network_sync_with_retry(&repository_name, "pulling latest main", || {
                    run_git_concurrent(
                        &path,
                        &["pull", "--ff-only", "origin", "main"],
                        &format!("pulling latest main for {repository_name}"),
                        &repository_name,
                    )
                    .map(|_| ())
                })?;
                info!("synchronized submodule {}", repository_name);
                Ok(())
            }
        }
    }
}

pub(super) fn run_network_sync_with_retry<Action>(
    repository_name: &str,
    operation: &str,
    mut action: Action,
) -> Result<()>
where
    Action: FnMut() -> Result<()>,
{
    for attempt in 1..=NETWORK_SYNC_MAX_ATTEMPTS {
        match action() {
            Ok(()) => return Ok(()),
            Err(error) if attempt < NETWORK_SYNC_MAX_ATTEMPTS => {
                let summary = error.to_string();
                let _ = log_concurrent_event(
                    repository_name,
                    ConcurrentLogState::Retrying,
                    &format!(
                        "{operation} failed on attempt {attempt}/{NETWORK_SYNC_MAX_ATTEMPTS}: {summary}; retrying"
                    ),
                );
                thread::sleep(Duration::from_millis(NETWORK_SYNC_RETRY_DELAY_MS));
            }
            Err(error) => return Err(error),
        }
    }

    anyhow::bail!("retry loop exited unexpectedly")
}

pub(super) fn prepare_repository_sync(
    root: &Path,
    repository: &ManagedRepository,
) -> Result<Option<PendingNetworkSync>> {
    let rel_path = repository.rel_path.as_str();
    let abs_path = root.join(rel_path);
    let registered = is_registered_submodule(root, rel_path)?;

    if abs_path.exists() && !registered {
        anyhow::bail!(
            "'{}' already exists but is not registered as a git submodule",
            rel_path
        );
    }

    if !registered {
        remove_stale_submodule_git_dir(root, rel_path)?;
        run_git(
            root,
            &["submodule", "add", "-b", "main", &repository.url, rel_path],
            &format!("adding submodule {}", repository.name),
            true,
        )?;
        info!("synchronized submodule {}", repository.name);
        return Ok(None);
    }

    run_git(
        root,
        &["submodule", "set-url", "--", rel_path, &repository.url],
        &format!("setting remote URL for {}", repository.name),
        false,
    )?;

    run_git(
        root,
        &[
            "submodule",
            "set-branch",
            "--branch",
            "main",
            "--",
            rel_path,
        ],
        &format!("tracking main for {}", repository.name),
        false,
    )?;

    if !is_initialized_submodule(&abs_path) {
        return Ok(Some(PendingNetworkSync::Initialize {
            root: root.to_path_buf(),
            repository: repository.clone(),
        }));
    }

    ensure_main_checked_out(&abs_path, &repository.name)?;
    Ok(Some(PendingNetworkSync::Pull {
        path: abs_path,
        repository_name: repository.name.clone(),
    }))
}

pub(super) fn execute_network_syncs(syncs: Vec<PendingNetworkSync>) -> Result<()> {
    if syncs.is_empty() {
        return Ok(());
    }

    if syncs.len() == 1 {
        return syncs.into_iter().next().unwrap().execute();
    }

    info!("synchronizing {} submodules in parallel", syncs.len());

    let mut errors = Vec::new();
    thread::scope(|scope| {
        let handles = syncs
            .into_iter()
            .map(|sync| {
                let repository_name = sync.repository_name().to_string();
                let handle = scope.spawn(move || sync.execute());
                (repository_name, handle)
            })
            .collect::<Vec<_>>();

        for (repository_name, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("{repository_name}: {error:#}")),
                Err(_) => errors.push(format!("{repository_name}: sync worker panicked")),
            }
        }
    });

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "failed to synchronize {} submodule(s): {}",
            errors.len(),
            errors.join("; ")
        );
    }
}
