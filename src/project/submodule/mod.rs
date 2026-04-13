mod cleanup;
mod git;
mod sync;

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::Path;

use self::cleanup::{remove_repository, repository_names_to_remove};
use self::git::{
    ensure_git_repository_root, managed_repository_names_from_gitmodules,
    repair_gitmodules_if_needed, tracked_repository_names_from_git_index,
};
use self::sync::{execute_pending_syncs, prepare_repository_sync};
use super::resolver::ManagedRepository;
use crate::project::network::NetworkBatchLogger;

#[derive(Debug)]
struct OnlineSyncRequired {
    repository_name: String,
}

impl OnlineSyncRequired {
    fn new(repository_name: impl Into<String>) -> Self {
        Self {
            repository_name: repository_name.into(),
        }
    }

    fn repository_name(&self) -> &str {
        &self.repository_name
    }
}

impl fmt::Display for OnlineSyncRequired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "repository '{}' is not present locally yet and requires an online `cpkg sync`",
            self.repository_name
        )
    }
}

impl StdError for OnlineSyncRequired {}

pub(crate) fn online_sync_required_error(repository_name: impl Into<String>) -> anyhow::Error {
    OnlineSyncRequired::new(repository_name).into()
}

pub(crate) fn online_sync_required_repository(error: &anyhow::Error) -> Option<&str> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<OnlineSyncRequired>()
            .map(OnlineSyncRequired::repository_name)
    })
}

pub fn sync_repositories(root: &Path, repositories: &[ManagedRepository]) -> Result<()> {
    sync_repositories_with_options(root, repositories, false)
}

pub fn sync_repositories_with_options(
    root: &Path,
    repositories: &[ManagedRepository],
    offline: bool,
) -> Result<()> {
    let network_logger = NetworkBatchLogger::new();
    ensure_git_repository_root(root)?;
    fs::create_dir_all(root.join("Modules")).context("failed to create Modules directory")?;
    let result = (|| -> Result<()> {
        repair_gitmodules_if_needed(root, repositories)?;

        let mut pending_syncs = Vec::new();
        for repository in repositories {
            if let Some(sync) = prepare_repository_sync(root, repository, offline, &network_logger)?
            {
                pending_syncs.push(sync);
            }
        }
        execute_pending_syncs(pending_syncs)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            network_logger.finish_success()?;
            Ok(())
        }
        Err(error) => {
            network_logger.finish_failure();
            Err(error)
        }
    }
}

pub fn remove_unused_repositories(
    root: &Path,
    current_repositories: &[String],
    desired_repositories: &[ManagedRepository],
) -> Result<()> {
    repair_gitmodules_if_needed(root, desired_repositories)?;
    let current_repositories = current_repositories
        .iter()
        .cloned()
        .chain(managed_repository_names_from_gitmodules(root)?)
        .chain(tracked_repository_names_from_git_index(root)?)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let repositories_to_remove =
        repository_names_to_remove(&current_repositories, desired_repositories);
    if repositories_to_remove.is_empty() {
        return Ok(());
    }

    ensure_git_repository_root(root)?;
    for repository_name in repositories_to_remove {
        remove_repository(root, &repository_name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cleanup::{remove_repository, repository_names_to_remove};
    use super::git::{
        current_branch, is_registered_submodule, submodule_git_dir, supports_offline_submodule_add,
    };
    use super::sync::{NETWORK_SYNC_MAX_ATTEMPTS, run_network_sync_with_retry};
    use super::{remove_unused_repositories, sync_repositories, sync_repositories_with_options};
    use crate::project::network::NetworkBatchLogger;
    use crate::project::resolver::ManagedRepository;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Once;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cpkg-submodule-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn run_git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "git command failed: git -C {:?} {:?}",
            path,
            args
        );
    }

    fn init_repo(path: &Path) {
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "cpkg@example.com"]);
        run_git(path, &["config", "user.name", "cpkg"]);
    }

    fn allow_file_protocol() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            std::env::set_var("GIT_ALLOW_PROTOCOL", "file");
        });
    }

    fn commit_file(path: &Path, name: &str, content: &str, message: &str) -> String {
        fs::write(path.join(name), content).unwrap();
        run_git(path, &["add", name]);
        run_git(
            path,
            &["-c", "commit.gpgsign=false", "commit", "-m", message],
        );
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap()
    }

    fn head_commit(path: &Path) -> String {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap()
    }

    fn gitmodules_contains_path(root: &Path, rel_path: &str) -> bool {
        let gitmodules = root.join(".gitmodules");
        gitmodules.exists()
            && fs::read_to_string(gitmodules)
                .map(|content| content.contains(rel_path))
                .unwrap_or(false)
    }

    fn write_broken_gitmodules(root: &Path) {
        fs::write(root.join(".gitmodules"), "<<<<<<< ours\n").unwrap();
    }

    #[test]
    fn repository_names_to_remove_keeps_referenced_repositories() {
        let repositories = repository_names_to_remove(
            &["MotorDrivers".to_string(), "BasicComponents".to_string()],
            &[ManagedRepository {
                name: "BasicComponents".to_string(),
                url: "https://example.com/BasicComponents.git".to_string(),
                rel_path: "Modules/BasicComponents".to_string(),
            }],
        );

        assert_eq!(repositories, vec!["MotorDrivers"]);
    }

    #[test]
    fn remove_repository_cleans_stale_gitdir_and_allows_readd() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        fs::write(origin.join("README.md"), "test").unwrap();
        run_git(&origin, &["add", "README.md"]);
        run_git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url.clone(),
            rel_path: rel_path.to_string(),
        };

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &origin_url,
                rel_path,
            ],
        );

        let git_dir = submodule_git_dir(&root, rel_path).unwrap();
        assert!(git_dir.exists());

        remove_repository(&root, "TrajectoryControl").unwrap();
        assert!(!root.join(rel_path).exists());
        assert!(!git_dir.exists());

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &repository.url,
                rel_path,
            ],
        );
        assert!(root.join(rel_path).exists());

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_repository_cleans_gitmodules_entry_when_gitlink_is_already_gone() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        fs::write(origin.join("README.md"), "test").unwrap();
        run_git(&origin, &["add", "README.md"]);
        run_git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &origin_url,
                rel_path,
            ],
        );

        let git_dir = submodule_git_dir(&root, rel_path).unwrap();
        assert!(git_dir.exists());
        assert!(is_registered_submodule(&root, rel_path).unwrap());

        run_git(&root, &["update-index", "--force-remove", rel_path]);
        fs::remove_dir_all(root.join(rel_path)).unwrap();

        remove_repository(&root, "TrajectoryControl").unwrap();

        assert!(!gitmodules_contains_path(&root, rel_path));
        assert!(!is_registered_submodule(&root, rel_path).unwrap());
        assert!(!git_dir.exists());

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_repository_succeeds_when_gitmodules_has_unstaged_changes() {
        allow_file_protocol();

        let origin_keep = make_temp_dir("origin-keep");
        let origin_remove = make_temp_dir("origin-remove");
        init_repo(&origin_keep);
        init_repo(&origin_remove);
        fs::write(origin_keep.join("README.md"), "keep").unwrap();
        fs::write(origin_remove.join("README.md"), "remove").unwrap();
        run_git(&origin_keep, &["add", "README.md"]);
        run_git(&origin_remove, &["add", "README.md"]);
        run_git(
            &origin_keep,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(
            &origin_remove,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin_keep, &["branch", "-M", "main"]);
        run_git(&origin_remove, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let keep_url = origin_keep
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let remove_url = origin_remove
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let keep_path = "Modules/BasicComponents";
        let remove_path = "Modules/Sensors";

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &keep_url,
                keep_path,
            ],
        );
        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &remove_url,
                remove_path,
            ],
        );

        run_git(
            &root,
            &[
                "submodule",
                "set-url",
                "--",
                keep_path,
                "git@example.com:BasicComponents.git",
            ],
        );

        remove_repository(&root, "Sensors").unwrap();

        assert!(gitmodules_contains_path(&root, keep_path));
        assert!(!gitmodules_contains_path(&root, remove_path));
        assert!(!root.join(remove_path).exists());

        let _ = fs::remove_dir_all(origin_keep);
        let _ = fs::remove_dir_all(origin_remove);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_unused_repositories_uses_gitmodules_when_generated_state_is_stale() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        fs::write(origin.join("README.md"), "test").unwrap();
        run_git(&origin, &["add", "README.md"]);
        run_git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &origin_url,
                rel_path,
            ],
        );

        remove_unused_repositories(&root, &[], &Vec::<ManagedRepository>::new()).unwrap();

        assert!(!root.join(rel_path).exists());
        assert!(!gitmodules_contains_path(&root, rel_path));
        assert!(!is_registered_submodule(&root, rel_path).unwrap());

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repositories_recovers_from_broken_gitmodules() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        fs::write(origin.join("README.md"), "test").unwrap();
        run_git(&origin, &["add", "README.md"]);
        run_git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url.clone(),
            rel_path: rel_path.to_string(),
        };

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &origin_url,
                rel_path,
            ],
        );

        write_broken_gitmodules(&root);
        sync_repositories(&root, &[repository]).unwrap();

        assert!(gitmodules_contains_path(&root, rel_path));
        assert!(is_registered_submodule(&root, rel_path).unwrap());

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_unused_repositories_recovers_from_broken_gitmodules_without_generated_state() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        fs::write(origin.join("README.md"), "test").unwrap();
        run_git(&origin, &["add", "README.md"]);
        run_git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/Sensors";

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &origin_url,
                rel_path,
            ],
        );

        write_broken_gitmodules(&root);
        remove_unused_repositories(&root, &[], &Vec::<ManagedRepository>::new()).unwrap();

        assert!(!root.join(rel_path).exists());

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repository_updates_uninitialized_submodule_to_latest_main() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        commit_file(&origin, "README.md", "v1\n", "init");
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url,
            rel_path: rel_path.to_string(),
        };

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &repository.url,
                rel_path,
            ],
        );

        let latest_commit = commit_file(&origin, "README.md", "v2\n", "update");

        run_git(&root, &["submodule", "deinit", "-f", "--", rel_path]);

        sync_repositories(&root, &[repository]).unwrap();

        assert_eq!(
            current_branch(&root.join(rel_path)).unwrap().as_deref(),
            Some("main")
        );

        let head = head_commit(&root.join(rel_path));
        assert_eq!(head, latest_commit);

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repository_switches_detached_submodule_back_to_main_before_pulling() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        commit_file(&origin, "README.md", "v1\n", "init");
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url,
            rel_path: rel_path.to_string(),
        };

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &repository.url,
                rel_path,
            ],
        );
        run_git(&root.join(rel_path), &["checkout", "--detach"]);

        let latest_commit = commit_file(&origin, "README.md", "v2\n", "update");

        sync_repositories(&root, &[repository]).unwrap();

        assert_eq!(
            current_branch(&root.join(rel_path)).unwrap().as_deref(),
            Some("main")
        );

        let head = head_commit(&root.join(rel_path));
        assert_eq!(head, latest_commit);

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repository_offline_keeps_initialized_submodule_on_cached_commit() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        commit_file(&origin, "README.md", "v1\n", "init");
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url,
            rel_path: rel_path.to_string(),
        };

        sync_repositories(&root, &[repository.clone()]).unwrap();
        let cached_commit = head_commit(&root.join(rel_path));
        let latest_commit = commit_file(&origin, "README.md", "v2\n", "update");

        sync_repositories_with_options(&root, &[repository], true).unwrap();

        assert_eq!(
            current_branch(&root.join(rel_path)).unwrap().as_deref(),
            Some("main")
        );
        assert_eq!(head_commit(&root.join(rel_path)), cached_commit);
        assert_ne!(cached_commit, latest_commit);

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repository_offline_initializes_submodule_from_local_cache() {
        allow_file_protocol();

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        commit_file(&origin, "README.md", "v1\n", "init");
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let origin_url = origin
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rel_path = "Modules/TrajectoryControl";
        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_url,
            rel_path: rel_path.to_string(),
        };

        sync_repositories(&root, &[repository.clone()]).unwrap();
        let cached_commit = head_commit(&root.join(rel_path));
        let latest_commit = commit_file(&origin, "README.md", "v2\n", "update");

        run_git(&root, &["submodule", "deinit", "-f", "--", rel_path]);
        sync_repositories_with_options(&root, &[repository], true).unwrap();

        assert_eq!(
            current_branch(&root.join(rel_path)).unwrap().as_deref(),
            Some("main")
        );
        assert_eq!(head_commit(&root.join(rel_path)), cached_commit);
        assert_ne!(cached_commit, latest_commit);

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repository_offline_reports_when_git_cannot_add_without_fetch() {
        allow_file_protocol();

        if supports_offline_submodule_add().unwrap() {
            return;
        }

        let origin = make_temp_dir("origin");
        init_repo(&origin);
        commit_file(&origin, "README.md", "v1\n", "init");
        run_git(&origin, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let repository = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: "Modules/TrajectoryControl".to_string(),
        };

        let error = sync_repositories_with_options(&root, &[repository], true).unwrap_err();
        assert!(error.to_string().contains(
            "repository 'TrajectoryControl' is not present locally yet and requires an online `cpkg sync`"
        ));

        let _ = fs::remove_dir_all(origin);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repositories_updates_registered_submodules_together() {
        allow_file_protocol();

        let origin_a = make_temp_dir("origin-a");
        let origin_b = make_temp_dir("origin-b");
        init_repo(&origin_a);
        init_repo(&origin_b);
        commit_file(&origin_a, "README.md", "a1\n", "init");
        commit_file(&origin_b, "README.md", "b1\n", "init");
        run_git(&origin_a, &["branch", "-M", "main"]);
        run_git(&origin_b, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let repository_a = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_a
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: "Modules/TrajectoryControl".to_string(),
        };
        let repository_b = ManagedRepository {
            name: "BasicComponents".to_string(),
            url: origin_b
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: "Modules/BasicComponents".to_string(),
        };

        for repository in [&repository_a, &repository_b] {
            run_git(
                &root,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-b",
                    "main",
                    &repository.url,
                    &repository.rel_path,
                ],
            );
        }

        let latest_a = commit_file(&origin_a, "README.md", "a2\n", "update");
        let latest_b = commit_file(&origin_b, "README.md", "b2\n", "update");
        run_git(
            &root,
            &["submodule", "deinit", "-f", "--", &repository_b.rel_path],
        );

        sync_repositories(&root, &[repository_a.clone(), repository_b.clone()]).unwrap();

        assert_eq!(
            current_branch(&root.join(&repository_a.rel_path))
                .unwrap()
                .as_deref(),
            Some("main")
        );
        assert_eq!(
            current_branch(&root.join(&repository_b.rel_path))
                .unwrap()
                .as_deref(),
            Some("main")
        );
        assert_eq!(head_commit(&root.join(&repository_a.rel_path)), latest_a);
        assert_eq!(head_commit(&root.join(&repository_b.rel_path)), latest_b);

        let _ = fs::remove_dir_all(origin_a);
        let _ = fs::remove_dir_all(origin_b);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_repositories_handles_mixed_added_and_updated_submodules() {
        allow_file_protocol();

        let origin_a = make_temp_dir("origin-a");
        let origin_b = make_temp_dir("origin-b");
        init_repo(&origin_a);
        init_repo(&origin_b);
        commit_file(&origin_a, "README.md", "a1\n", "init");
        commit_file(&origin_b, "README.md", "b1\n", "init");
        run_git(&origin_a, &["branch", "-M", "main"]);
        run_git(&origin_b, &["branch", "-M", "main"]);

        let root = make_temp_dir("root");
        init_repo(&root);
        fs::create_dir_all(root.join("Modules")).unwrap();

        let repository_a = ManagedRepository {
            name: "TrajectoryControl".to_string(),
            url: origin_a
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: "Modules/TrajectoryControl".to_string(),
        };
        let repository_b = ManagedRepository {
            name: "BasicComponents".to_string(),
            url: origin_b
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: "Modules/BasicComponents".to_string(),
        };

        run_git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-b",
                "main",
                &repository_a.url,
                &repository_a.rel_path,
            ],
        );

        let latest_a = commit_file(&origin_a, "README.md", "a2\n", "update");
        let latest_b = commit_file(&origin_b, "README.md", "b2\n", "update");

        sync_repositories(&root, &[repository_a.clone(), repository_b.clone()]).unwrap();

        assert_eq!(
            current_branch(&root.join(&repository_a.rel_path))
                .unwrap()
                .as_deref(),
            Some("main")
        );
        assert_eq!(
            current_branch(&root.join(&repository_b.rel_path))
                .unwrap()
                .as_deref(),
            Some("main")
        );
        assert_eq!(head_commit(&root.join(&repository_a.rel_path)), latest_a);
        assert_eq!(head_commit(&root.join(&repository_b.rel_path)), latest_b);

        let _ = fs::remove_dir_all(origin_a);
        let _ = fs::remove_dir_all(origin_b);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_network_sync_with_retry_retries_once_before_success() {
        let mut attempts = 0;
        let logger = NetworkBatchLogger::new();

        let result = run_network_sync_with_retry(
            "TrajectoryControl",
            "pulling latest main",
            &logger,
            || {
                attempts += 1;
                if attempts < NETWORK_SYNC_MAX_ATTEMPTS {
                    anyhow::bail!("temporary network error")
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_ok());
        assert_eq!(attempts, NETWORK_SYNC_MAX_ATTEMPTS);
    }

    #[test]
    fn run_network_sync_with_retry_returns_last_error_after_exhausting_attempts() {
        let mut attempts = 0;
        let logger = NetworkBatchLogger::new();

        let error = run_network_sync_with_retry(
            "TrajectoryControl",
            "pulling latest main",
            &logger,
            || {
                attempts += 1;
                anyhow::bail!("permanent network error")
            },
        )
        .unwrap_err();

        assert_eq!(attempts, NETWORK_SYNC_MAX_ATTEMPTS);
        assert!(error.to_string().contains("permanent network error"));
    }
}
