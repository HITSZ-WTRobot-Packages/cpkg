use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

use crate::project::network::{run_logged_command, run_logged_command_concurrent};

pub(super) fn run_git(
    root: &Path,
    args: &[&str],
    description: &str,
    show_logs: bool,
) -> Result<String> {
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

pub(super) fn run_git_concurrent(
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

pub(super) fn ensure_git_repository_root(root: &Path) -> Result<()> {
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

pub(super) fn is_initialized_submodule(path: &Path) -> bool {
    path.join(".git").exists()
}

pub(super) fn current_branch(path: &Path) -> Result<Option<String>> {
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

pub(super) fn ensure_main_checked_out(path: &Path, repository_name: &str) -> Result<()> {
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

pub(super) fn align_main_to_origin(path: &Path, repository_name: &str) -> Result<()> {
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

pub(super) fn is_registered_submodule(root: &Path, rel_path: &str) -> Result<bool> {
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

pub(super) fn submodule_git_dir(root: &Path, rel_path: &str) -> Result<PathBuf> {
    let mut path = git_common_dir(root)?.join("modules");
    for component in Path::new(rel_path).components() {
        path.push(component);
    }
    Ok(path)
}

pub(super) fn remove_stale_submodule_git_dir(root: &Path, rel_path: &str) -> Result<()> {
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
