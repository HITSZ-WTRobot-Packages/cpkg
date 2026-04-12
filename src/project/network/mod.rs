mod format;
mod panel;

use anyhow::{Context, Result};
use console::{Term, style};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use self::format::{colorize_prefix, colorize_state, error_summary, format_command};
use self::panel::{TransientLogPanel, join_reader, read_stream};

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

fn concurrent_output_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write_concurrent_log_line(line: &str) -> Result<()> {
    let _guard = concurrent_output_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("network log lock poisoned"))?;
    Term::stderr().write_line(line)?;
    Ok(())
}

pub fn log_concurrent_event(prefix: &str, state: ConcurrentLogState, message: &str) -> Result<()> {
    write_concurrent_log_line(&format!(
        "[{}][{}] {}: {}",
        style("network").dim(),
        colorize_prefix(prefix),
        colorize_state(state),
        message
    ))
}

pub fn run_logged_command(command: &mut Command, title: &str) -> Result<LoggedCommandOutput> {
    let command_display = format_command(command);
    let mut panel = TransientLogPanel::new(title, &command_display)?;

    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("Failed to start command: {error}");
            let _ = panel.finish_failure(&message);
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
        panel.push_line(event.kind, &event.line)?;
    }

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {title}"))?;
    let stdout = join_reader(stdout_handle, "stdout")?;
    let stderr = join_reader(stderr_handle, "stderr")?;

    if status.success() {
        panel.finish_success()?;
        Ok(LoggedCommandOutput { stdout, stderr })
    } else {
        let message = error_summary(title, status, &stderr, &stdout);
        let _ = panel.finish_failure(&message);
        anyhow::bail!("{message}");
    }
}

pub fn run_logged_command_concurrent(
    command: &mut Command,
    title: &str,
    prefix: &str,
) -> Result<LoggedCommandOutput> {
    let command_display = format_command(command);
    log_concurrent_event(prefix, ConcurrentLogState::Started, title)?;

    let output = command
        .output()
        .with_context(|| format!("failed to execute {title}"))?;
    let status = output.status;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if status.success() {
        log_concurrent_event(prefix, ConcurrentLogState::Completed, title)?;
        Ok(LoggedCommandOutput { stdout, stderr })
    } else {
        let message = error_summary(title, status, &stderr, &stdout);
        let _ = log_concurrent_event(prefix, ConcurrentLogState::Failed, &message);
        let _ = write_concurrent_log_line(&format!(
            "  [{}] {}",
            colorize_prefix(prefix),
            style(format!("$ {command_display}")).dim()
        ));
        anyhow::bail!("{message}");
    }
}
