use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::info;

use crate::resolver::ManagedRepository;

fn run_git(root: &Path, args: &[&str], description: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
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

fn ensure_git_repository_root(root: &Path) -> Result<()> {
    let toplevel = run_git(
        root,
        &["rev-parse", "--show-toplevel"],
        "checking repository root",
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
        run_git(
            root,
            &["submodule", "add", "-b", "main", &repository.url, rel_path],
            &format!("adding submodule {}", repository.name),
        )?;
    }

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
    run_git(
        root,
        &["submodule", "update", "--init", "--remote", "--", rel_path],
        &format!("updating submodule {}", repository.name),
    )?;

    let abs_path_string = abs_path.to_string_lossy().into_owned();
    run_git(
        root,
        &["-C", &abs_path_string, "checkout", "main"],
        &format!("checking out main for {}", repository.name),
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
    )?;

    info!("synchronized submodule {}", repository.name);
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
