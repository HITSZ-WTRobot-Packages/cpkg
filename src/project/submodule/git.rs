use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

use crate::project::network::{run_logged_command, run_logged_command_concurrent};
use crate::project::resolver::ManagedRepository;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredSubmodule {
    section: String,
    path: String,
}

fn registered_submodules(root: &Path) -> Result<Vec<RegisteredSubmodule>> {
    let gitmodules = root.join(".gitmodules");
    if !gitmodules.exists() {
        return Ok(Vec::new());
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

    if !output.status.success() {
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(Vec::new());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "failed to inspect .gitmodules: {}",
            if stderr.is_empty() {
                "unknown git error".to_string()
            } else {
                stderr
            }
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("failed to parse .gitmodules key from '{line}'"))?;
            let path = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("failed to parse .gitmodules path from '{line}'"))?;
            let section = key
                .strip_suffix(".path")
                .ok_or_else(|| anyhow::anyhow!("unexpected .gitmodules key '{key}'"))?;
            Ok(RegisteredSubmodule {
                section: section.to_string(),
                path: path.to_string(),
            })
        })
        .collect()
}

fn tracked_submodule_paths(root: &Path) -> Result<Vec<String>> {
    if !root.join(".git").exists() {
        return Ok(Vec::new());
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--stage"])
        .output()
        .context("failed to inspect tracked submodules from git index")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "failed to inspect tracked submodules from git index: {}",
            if stderr.is_empty() {
                "unknown git error".to_string()
            } else {
                stderr
            }
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let mode = parts.next()?;
            if mode != "160000" {
                return None;
            }
            let _object = parts.next()?;
            let _stage = parts.next()?;
            parts.next().map(|path| path.to_string())
        })
        .collect())
}

fn write_gitmodules_entry(root: &Path, repository: &ManagedRepository) -> Result<()> {
    let section = format!("submodule.{}", repository.name);

    run_git(
        root,
        &[
            "config",
            "--file",
            ".gitmodules",
            &format!("{section}.path"),
            &repository.rel_path,
        ],
        &format!("restoring .gitmodules path for {}", repository.name),
        false,
    )?;
    run_git(
        root,
        &[
            "config",
            "--file",
            ".gitmodules",
            &format!("{section}.url"),
            &repository.url,
        ],
        &format!("restoring .gitmodules url for {}", repository.name),
        false,
    )?;
    run_git(
        root,
        &[
            "config",
            "--file",
            ".gitmodules",
            &format!("{section}.branch"),
            "main",
        ],
        &format!("restoring .gitmodules branch for {}", repository.name),
        false,
    )?;

    Ok(())
}

pub(super) fn repair_gitmodules_if_needed(
    root: &Path,
    desired_repositories: &[ManagedRepository],
) -> Result<()> {
    if !root.join(".git").exists() {
        return Ok(());
    }

    if registered_submodules(root).is_ok() {
        return Ok(());
    }

    info!("detected invalid .gitmodules; rebuilding managed submodule entries");
    fs::write(root.join(".gitmodules"), "").context("failed to reset broken .gitmodules")?;

    for repository in desired_repositories {
        if has_git_index_entry(root, &repository.rel_path)? {
            write_gitmodules_entry(root, repository)?;
        }
    }

    Ok(())
}

pub(super) fn managed_repository_names_from_gitmodules(root: &Path) -> Result<Vec<String>> {
    let mut repositories = BTreeSet::new();

    for submodule in registered_submodules(root)? {
        let mut components = Path::new(&submodule.path).components();
        let Some(prefix) = components.next() else {
            continue;
        };
        let Some(repository_name) = components.next() else {
            continue;
        };
        if prefix.as_os_str() != "Modules" || components.next().is_some() {
            continue;
        }

        repositories.insert(repository_name.as_os_str().to_string_lossy().into_owned());
    }

    Ok(repositories.into_iter().collect())
}

pub(super) fn tracked_repository_names_from_git_index(root: &Path) -> Result<Vec<String>> {
    let mut repositories = BTreeSet::new();

    for path in tracked_submodule_paths(root)? {
        let mut components = Path::new(&path).components();
        let Some(prefix) = components.next() else {
            continue;
        };
        let Some(repository_name) = components.next() else {
            continue;
        };
        if prefix.as_os_str() != "Modules" || components.next().is_some() {
            continue;
        }

        repositories.insert(repository_name.as_os_str().to_string_lossy().into_owned());
    }

    Ok(repositories.into_iter().collect())
}

pub(super) fn is_registered_submodule(root: &Path, rel_path: &str) -> Result<bool> {
    Ok(registered_submodules(root)?
        .iter()
        .any(|submodule| submodule.path == rel_path))
}

pub(super) fn remove_submodule_registration(root: &Path, rel_path: &str) -> Result<bool> {
    let Some(section) = registered_submodules(root)?
        .into_iter()
        .find(|submodule| submodule.path == rel_path)
        .map(|submodule| submodule.section)
    else {
        return Ok(false);
    };

    run_git(
        root,
        &[
            "config",
            "--file",
            ".gitmodules",
            "--remove-section",
            &section,
        ],
        &format!("removing .gitmodules entry for {rel_path}"),
        false,
    )?;

    Ok(true)
}

pub(super) fn has_git_index_entry(root: &Path, rel_path: &str) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--stage", "--", rel_path])
        .output()
        .with_context(|| format!("failed to inspect git index entry for '{rel_path}'"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "failed to inspect git index entry for '{}': {}",
            rel_path,
            if stderr.is_empty() {
                "unknown git error".to_string()
            } else {
                stderr
            }
        );
    }

    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
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
