use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

use super::network::run_logged_command;
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

fn sync_repository(root: &Path, repository: &ManagedRepository) -> Result<()> {
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
    run_git(
        root,
        &["submodule", "update", "--init", "--remote", "--", rel_path],
        &format!("updating submodule {}", repository.name),
        true,
    )?;

    let abs_path_string = abs_path.to_string_lossy().into_owned();
    run_git(
        root,
        &["-C", &abs_path_string, "checkout", "main"],
        &format!("checking out main for {}", repository.name),
        false,
    )?;
    run_git(
        root,
        &[
            "-C",
            &abs_path_string,
            "pull",
            "--ff-only",
            "origin",
            "main",
        ],
        &format!("pulling latest main for {}", repository.name),
        true,
    )?;

    info!("synchronized submodule {}", repository.name);
    Ok(())
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

    for repository in repositories {
        sync_repository(root, repository)?;
    }

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
    use super::{remove_repository, repository_names_to_remove, submodule_git_dir};
    use crate::project::resolver::ManagedRepository;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
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
}
