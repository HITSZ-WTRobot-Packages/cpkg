use anyhow::Result;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tracing::info;

use super::git::{
    align_main_to_origin, ensure_main_checked_out, is_initialized_submodule,
    is_registered_submodule, remove_stale_submodule_git_dir, run_git, run_git_network,
    supports_offline_submodule_add,
};
use super::online_sync_required_error;
use crate::project::network::NetworkBatchLogger;
use crate::project::resolver::ManagedRepository;

pub(super) const NETWORK_SYNC_MAX_ATTEMPTS: usize = 2;
const NETWORK_SYNC_RETRY_DELAY_MS: u64 = 300;

#[derive(Debug, Clone)]
pub(super) enum PendingSubmoduleSync {
    Initialize {
        root: PathBuf,
        repository: ManagedRepository,
        logger: NetworkBatchLogger,
    },
    InitializeOffline {
        root: PathBuf,
        repository: ManagedRepository,
    },
    Pull {
        path: PathBuf,
        repository_name: String,
        logger: NetworkBatchLogger,
    },
}

impl PendingSubmoduleSync {
    fn repository_name(&self) -> &str {
        match self {
            Self::Initialize { repository, .. } => &repository.name,
            Self::InitializeOffline { repository, .. } => &repository.name,
            Self::Pull {
                repository_name, ..
            } => repository_name,
        }
    }

    fn initializes_submodule(&self) -> bool {
        matches!(
            self,
            Self::Initialize { .. } | Self::InitializeOffline { .. }
        )
    }

    fn execute(self) -> Result<()> {
        match self {
            Self::Initialize {
                root,
                repository,
                logger,
            } => {
                let rel_path = repository.rel_path.as_str();
                let abs_path = root.join(rel_path);
                run_network_sync_with_retry(
                    &repository.name,
                    "initializing submodule",
                    &logger,
                    || {
                        run_git_network(
                            &root,
                            &["submodule", "update", "--init", "--remote", "--", rel_path],
                            &format!("initializing submodule {}", repository.name),
                            &repository.name,
                            &logger,
                        )
                        .map(|_| ())
                    },
                )?;
                align_main_to_origin(&abs_path, &repository.name)?;
                info!("synchronized submodule {}", repository.name);
                Ok(())
            }
            Self::InitializeOffline { root, repository } => {
                let rel_path = repository.rel_path.as_str();
                let abs_path = root.join(rel_path);
                run_git(
                    &root,
                    &[
                        "submodule",
                        "update",
                        "--init",
                        "--remote",
                        "--no-fetch",
                        "--",
                        rel_path,
                    ],
                    &format!(
                        "initializing submodule {} from local cache",
                        repository.name
                    ),
                )?;
                align_main_to_origin(&abs_path, &repository.name)?;
                info!("initialized submodule {} from local cache", repository.name);
                Ok(())
            }
            Self::Pull {
                path,
                repository_name,
                logger,
            } => {
                run_network_sync_with_retry(
                    &repository_name,
                    "pulling latest main",
                    &logger,
                    || {
                        run_git_network(
                            &path,
                            &["pull", "--ff-only", "origin", "main"],
                            &format!("pulling latest main for {repository_name}"),
                            &repository_name,
                            &logger,
                        )
                        .map(|_| ())
                    },
                )?;
                info!("synchronized submodule {}", repository_name);
                Ok(())
            }
        }
    }
}

pub(super) fn run_network_sync_with_retry<Action>(
    repository_name: &str,
    operation: &str,
    logger: &NetworkBatchLogger,
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
                let _ = logger.log_retry(
                    repository_name,
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

fn add_submodule_without_fetch(root: &Path, repository: &ManagedRepository) -> Result<()> {
    if !supports_offline_submodule_add()? {
        return Err(online_sync_required_error(repository.name.clone()));
    }

    run_git(
        root,
        &[
            "submodule",
            "add",
            "--no-fetch",
            "-b",
            "main",
            &repository.url,
            repository.rel_path.as_str(),
        ],
        &format!("adding submodule {} without fetching", repository.name),
    )?;
    info!(
        "registered submodule {} without fetching repository data",
        repository.name
    );
    Ok(())
}

pub(super) fn prepare_repository_sync(
    root: &Path,
    repository: &ManagedRepository,
    offline: bool,
    logger: &NetworkBatchLogger,
) -> Result<Option<PendingSubmoduleSync>> {
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
        if offline {
            add_submodule_without_fetch(root, repository)?;
        } else {
            run_git_network(
                root,
                &["submodule", "add", "-b", "main", &repository.url, rel_path],
                &format!("adding submodule {}", repository.name),
                &repository.name,
                logger,
            )?;
            info!("synchronized submodule {}", repository.name);
        }
        return Ok(None);
    }

    run_git(
        root,
        &["submodule", "set-url", "--", rel_path, &repository.url],
        &format!("setting remote URL for {}", repository.name),
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
    )?;

    if !is_initialized_submodule(&abs_path) {
        return Ok(Some(if offline {
            PendingSubmoduleSync::InitializeOffline {
                root: root.to_path_buf(),
                repository: repository.clone(),
            }
        } else {
            PendingSubmoduleSync::Initialize {
                root: root.to_path_buf(),
                repository: repository.clone(),
                logger: logger.clone(),
            }
        }));
    }

    ensure_main_checked_out(&abs_path, &repository.name)?;
    if offline {
        info!("using cached submodule state for {}", repository.name);
        return Ok(None);
    }

    Ok(Some(PendingSubmoduleSync::Pull {
        path: abs_path,
        repository_name: repository.name.clone(),
        logger: logger.clone(),
    }))
}

pub(super) fn execute_pending_syncs(syncs: Vec<PendingSubmoduleSync>) -> Result<()> {
    let mut parallel_syncs = Vec::new();
    for sync in syncs {
        if sync.initializes_submodule() {
            sync.execute()?;
        } else {
            parallel_syncs.push(sync);
        }
    }

    if parallel_syncs.is_empty() {
        return Ok(());
    }

    if parallel_syncs.len() == 1 {
        return parallel_syncs.into_iter().next().unwrap().execute();
    }

    info!(
        "synchronizing {} submodules in parallel",
        parallel_syncs.len()
    );

    let mut errors = Vec::new();
    thread::scope(|scope| {
        let handles = parallel_syncs
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
