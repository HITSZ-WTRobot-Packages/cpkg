use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use tracing::info;

use super::network::{run_logged_command, run_logged_command_concurrent};
use super::resolver::ManagedRepository;

fn run_git(root: &Path, args: &[&str], description: &str, show_logs: bool) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);

    if show_logs {
        let output = run_logged_command(&mut command, description)
            .with_context(|| format!("failed to run git for {description}"))?;
        Ok(output.stdout.trim().to_string())
    } else {
        let output = command
            .output()
            .with_context(|| format!("failed to run git for {}", description))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "git failed while {}: {}",
                description,
                if stderr.is_empty() {
                    "unknown git error".to_string()
                } else {
                    stderr
                }
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn run_git_concurrent(
    root: &Path,
    args: &[&str],
    description: &str,
    prefix: &str,
) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    let output = run_logged_command_concurrent(&mut command, description, prefix)
        .with_context(|| format!("failed to run git for {description}"))?;
    Ok(output.stdout.trim().to_string())
}

fn ensure_git_repository_root(root: &Path) -> Result<()> {
    let toplevel = run_git(
        root,
        &["rev-parse", "--show-toplevel"],
        "checking repository root",
        false,
    )?;
    let canonical_root = fs::canonicalize(root).context("failed to resolve current directory")?;
    let canonical_toplevel =
        fs::canonicalize(toplevel).context("failed to resolve git repository root")?;

    if canonical_root != canonical_toplevel {
        anyhow::bail!("run cpkg from the git repository root that contains `wtrproject.toml`");
    }

    Ok(())
}

fn is_initialized_submodule(path: &Path) -> bool {
    path.join(".git").exists()
}

fn current_branch(path: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .with_context(|| format!("failed to read current branch for '{}'", path.display()))?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    if output.status.code() == Some(1) {
        return Ok(None);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "git failed while reading current branch for '{}': {}",
        path.display(),
        if stderr.is_empty() {
            "unknown git error".to_string()
        } else {
            stderr
        }
    );
}

fn has_local_branch(path: &Path, branch: &str) -> Result<bool> {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .with_context(|| {
            format!(
                "failed to check whether branch '{branch}' exists in '{}'",
                path.display()
            )
        })?;

    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "git failed while checking branch '{branch}' in '{}'",
            path.display()
        ),
    }
}

fn ensure_main_checked_out(path: &Path, repository_name: &str) -> Result<()> {
    if current_branch(path)?.as_deref() == Some("main") {
        return Ok(());
    }

    if has_local_branch(path, "main")? {
        run_git(
            path,
            &["checkout", "main"],
            &format!("checking out main for {repository_name}"),
            false,
        )?;
    } else {
        run_git(
            path,
            &["checkout", "-b", "main", "--track", "origin/main"],
            &format!("creating main for {repository_name}"),
            false,
        )?;
    }

    Ok(())
}

fn align_main_to_origin(path: &Path, repository_name: &str) -> Result<()> {
    run_git(
        path,
        &["branch", "-f", "main", "origin/main"],
        &format!("aligning main to origin/main for {repository_name}"),
        false,
    )?;
    run_git(
        path,
        &["checkout", "main"],
        &format!("checking out main for {repository_name}"),
        false,
    )?;
    run_git(
        path,
        &["branch", "--set-upstream-to", "origin/main", "main"],
        &format!("tracking origin/main for {repository_name}"),
        false,
    )?;
    Ok(())
}

fn is_registered_submodule(root: &Path, rel_path: &str) -> Result<bool> {
    let gitmodules = root.join(".gitmodules");
    if !gitmodules.exists() {
        return Ok(false);
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "config",
            "--file",
            ".gitmodules",
            "--get-regexp",
            "^submodule\\..*\\.path$",
        ])
        .output()
        .context("failed to inspect .gitmodules")?;

    if !output.status.success() && output.stdout.is_empty() {
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .map(|value| value == rel_path)
            .unwrap_or(false)
    }))
}

fn git_common_dir(root: &Path) -> Result<PathBuf> {
    let git_dir = run_git(
        root,
        &["rev-parse", "--git-common-dir"],
        "resolving git dir",
        false,
    )?;
    let git_dir = PathBuf::from(git_dir);

    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(root.join(git_dir))
    }
}

fn submodule_git_dir(root: &Path, rel_path: &str) -> Result<PathBuf> {
    let mut path = git_common_dir(root)?.join("modules");
    for component in Path::new(rel_path).components() {
        path.push(component);
    }
    Ok(path)
}

fn remove_stale_submodule_git_dir(root: &Path, rel_path: &str) -> Result<()> {
    let git_dir = submodule_git_dir(root, rel_path)?;
    if !git_dir.exists() {
        return Ok(());
    }

    fs::remove_dir_all(&git_dir).with_context(|| {
        format!(
            "failed to remove stale submodule git directory '{}'",
            git_dir.display()
        )
    })?;
    info!(
        "removed stale submodule git directory {}",
        git_dir.display()
    );
    Ok(())
}

#[derive(Debug, Clone)]
enum PendingNetworkSync {
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
                run_git_concurrent(
                    &root,
                    &["submodule", "update", "--init", "--remote", "--", rel_path],
                    &format!("initializing submodule {}", repository.name),
                    &repository.name,
                )?;
                align_main_to_origin(&abs_path, &repository.name)?;
                info!("synchronized submodule {}", repository.name);
                Ok(())
            }
            Self::Pull {
                path,
                repository_name,
            } => {
                run_git_concurrent(
                    &path,
                    &["pull", "--ff-only", "origin", "main"],
                    &format!("pulling latest main for {repository_name}"),
                    &repository_name,
                )?;
                info!("synchronized submodule {}", repository_name);
                Ok(())
            }
        }
    }
}

fn prepare_repository_sync(
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

fn execute_network_syncs(syncs: Vec<PendingNetworkSync>) -> Result<()> {
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

fn repository_names_to_remove(
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

fn remove_repository(root: &Path, repository_name: &str) -> Result<()> {
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

pub fn sync_repositories(root: &Path, repositories: &[ManagedRepository]) -> Result<()> {
    ensure_git_repository_root(root)?;
    fs::create_dir_all(root.join("Modules")).context("failed to create Modules directory")?;

    let mut pending_syncs = Vec::new();
    for repository in repositories {
        if let Some(sync) = prepare_repository_sync(root, repository)? {
            pending_syncs.push(sync);
        }
    }
    execute_network_syncs(pending_syncs)?;

    Ok(())
}

pub fn remove_unused_repositories(
    root: &Path,
    current_repositories: &[String],
    desired_repositories: &[ManagedRepository],
) -> Result<()> {
    let repositories_to_remove =
        repository_names_to_remove(current_repositories, desired_repositories);
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
    use super::{
        current_branch, remove_repository, repository_names_to_remove, submodule_git_dir,
        sync_repositories,
    };
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
}
