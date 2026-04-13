mod format;
mod panel;

use anyhow::{Context, Result};
use std::process::{Command, Stdio};

use self::format::{error_summary, format_command};
use self::panel::{join_reader, read_stream};

pub(crate) use self::panel::NetworkBatchLogger;

pub struct LoggedCommandOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrentLogState {
    Started,
    Completed,
    Failed,
    Retrying,
}

pub(crate) fn run_logged_command_in_batch(
    command: &mut Command,
    title: &str,
    label: &str,
    logger: &NetworkBatchLogger,
) -> Result<LoggedCommandOutput> {
    let command_display = format_command(command);
    let task = logger.start_task(label, title, &command_display)?;

    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("Failed to start command: {error}");
            let _ = task.finish_failure(&message);
            return Err(error).with_context(|| format!("failed to spawn {title}"));
        }
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture stdout for {title}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture stderr for {title}"))?;

    let (sender, receiver) = std::sync::mpsc::channel();
    let stdout_handle = read_stream(stdout, panel::StreamKind::Stdout, sender.clone());
    let stderr_handle = read_stream(stderr, panel::StreamKind::Stderr, sender);

    for event in receiver {
        task.push_line(event.kind, &event.line)?;
    }

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {title}"))?;
    let stdout = join_reader(stdout_handle, "stdout")?;
    let stderr = join_reader(stderr_handle, "stderr")?;

    if status.success() {
        task.finish_success()?;
        Ok(LoggedCommandOutput { stdout, stderr })
    } else {
        let message = error_summary(title, status, &stderr, &stdout);
        let _ = task.finish_failure(&message);
        anyhow::bail!("{message}");
    }
}
