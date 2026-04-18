use anyhow::{Context, Result};
use console::{Term, style};
use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use tracing::debug;

use super::ConcurrentLogState;
use super::format::{colorize_prefix, colorize_state, fit_to_width, state_label};

pub(super) const PANEL_MAX_LOG_LINES: usize = 6;
const PANEL_MAX_TASK_LINES: usize = 4;

#[derive(Clone, Copy)]
pub(super) enum StreamKind {
    Stdout,
    Stderr,
}

pub(super) struct StreamEvent {
    pub(super) kind: StreamKind,
    pub(super) line: String,
}

#[derive(Debug, Clone)]
struct GroupedTask {
    id: usize,
    label: String,
    title: String,
    command: String,
    state: ConcurrentLogState,
    status_message: Option<String>,
    recent_lines: Vec<String>,
    debug_lines: Vec<String>,
}

impl GroupedTask {
    fn new(id: usize, label: &str, title: &str, command: &str) -> Self {
        Self {
            id,
            label: label.to_string(),
            title: title.to_string(),
            command: command.to_string(),
            state: ConcurrentLogState::Started,
            status_message: None,
            recent_lines: Vec::new(),
            debug_lines: Vec::new(),
        }
    }

    fn push_line(&mut self, kind: StreamKind, line: &str) {
        let prefix = match kind {
            StreamKind::Stdout => "stdout",
            StreamKind::Stderr => "stderr",
        };
        let formatted = format!("{prefix}: {line}");
        self.recent_lines.push(formatted.clone());
        self.debug_lines.push(formatted);
        if self.recent_lines.len() > PANEL_MAX_LOG_LINES {
            let overflow = self.recent_lines.len() - PANEL_MAX_LOG_LINES;
            self.recent_lines.drain(0..overflow);
        }
    }

    fn set_state(&mut self, state: ConcurrentLogState, message: Option<String>) {
        self.state = state;
        self.status_message = message;
    }

    fn single_status_line(&self) -> String {
        match self.state {
            ConcurrentLogState::Started => "Running network request...".to_string(),
            ConcurrentLogState::Completed => "Completed.".to_string(),
            ConcurrentLogState::Retrying => self
                .status_message
                .clone()
                .unwrap_or_else(|| "Retrying network request...".to_string()),
            ConcurrentLogState::Failed => self
                .status_message
                .clone()
                .unwrap_or_else(|| "Network request failed.".to_string()),
        }
    }

    fn summary_line(&self) -> String {
        let message = match self.state {
            ConcurrentLogState::Started | ConcurrentLogState::Completed => self.title.clone(),
            ConcurrentLogState::Retrying | ConcurrentLogState::Failed => self
                .status_message
                .clone()
                .unwrap_or_else(|| self.title.clone()),
        };

        format!("[{}] {}: {}", self.label, state_label(self.state), message)
    }

    fn is_active(&self) -> bool {
        matches!(
            self.state,
            ConcurrentLogState::Started | ConcurrentLogState::Retrying
        )
    }
}

#[derive(Debug, Clone)]
struct CompletedTaskDebugSnapshot {
    label: String,
    title: String,
    command: String,
    state: ConcurrentLogState,
    status_message: Option<String>,
    log_lines: Vec<String>,
}

impl From<&GroupedTask> for CompletedTaskDebugSnapshot {
    fn from(task: &GroupedTask) -> Self {
        Self {
            label: task.label.clone(),
            title: task.title.clone(),
            command: task.command.clone(),
            state: task.state,
            status_message: task.status_message.clone(),
            log_lines: task.debug_lines.clone(),
        }
    }
}

fn task_completion_debug_message(task: &CompletedTaskDebugSnapshot) -> String {
    match task.state {
        ConcurrentLogState::Started | ConcurrentLogState::Completed => task.title.clone(),
        ConcurrentLogState::Retrying | ConcurrentLogState::Failed => task
            .status_message
            .clone()
            .unwrap_or_else(|| task.title.clone()),
    }
}

fn emit_task_completion_debug_logs(task: &CompletedTaskDebugSnapshot) {
    debug!(
        target: "cpkg::network",
        label = %task.label,
        state = state_label(task.state),
        title = %task.title,
        command = %task.command,
        summary = %task_completion_debug_message(task),
        "network task finished"
    );

    if task.log_lines.is_empty() {
        debug!(
            target: "cpkg::network",
            label = %task.label,
            "no network task output captured"
        );
        return;
    }

    for line in &task.log_lines {
        debug!(target: "cpkg::network", label = %task.label, "{line}");
    }
}

#[derive(Debug)]
struct GroupedLogState {
    is_tty: bool,
    cursor_hidden: bool,
    rendered_lines: usize,
    next_task_id: usize,
    parallel_task_mode: bool,
    tasks: Vec<GroupedTask>,
    recent_lines: Vec<String>,
    last_updated_task_id: Option<usize>,
}

impl GroupedLogState {
    fn new() -> Self {
        Self {
            is_tty: Term::stderr().is_term(),
            cursor_hidden: false,
            rendered_lines: 0,
            next_task_id: 0,
            parallel_task_mode: false,
            tasks: Vec::new(),
            recent_lines: Vec::new(),
            last_updated_task_id: None,
        }
    }

    fn start_task(&mut self, label: &str, title: &str, command: &str) -> Result<usize> {
        let task_id = self.next_task_id;
        self.next_task_id += 1;
        self.last_updated_task_id = Some(task_id);
        self.tasks
            .push(GroupedTask::new(task_id, label, title, command));

        if self.is_tty {
            self.ensure_cursor_hidden()?;
            self.render()?;
        } else {
            write_grouped_event(label, ConcurrentLogState::Started, title)?;
            Term::stderr().write_line(&format!(
                "  [{}] {}",
                colorize_prefix(label),
                style(format!("$ {command}")).dim()
            ))?;
        }

        Ok(task_id)
    }

    fn push_task_line(&mut self, task_id: usize, kind: StreamKind, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }

        let stream = match kind {
            StreamKind::Stdout => "stdout",
            StreamKind::Stderr => "stderr",
        };
        let Some(label) = ({
            let Some(task) = self.task_mut(task_id) else {
                return Ok(());
            };
            task.push_line(kind, line);
            Some(task.label.clone())
        }) else {
            return Ok(());
        };
        self.last_updated_task_id = Some(task_id);

        self.recent_lines
            .push(format!("[{}][{}] {}", label, stream, line));
        if self.recent_lines.len() > PANEL_MAX_LOG_LINES {
            let overflow = self.recent_lines.len() - PANEL_MAX_LOG_LINES;
            self.recent_lines.drain(0..overflow);
        }

        if self.is_tty {
            self.render()?;
        } else {
            Term::stderr().write_line(&format!(
                "  [{}][{}] {}",
                colorize_prefix(&label),
                stream,
                line
            ))?;
        }

        Ok(())
    }

    fn set_parallel_task_mode(&mut self, enabled: bool) {
        self.parallel_task_mode = enabled;
    }

    fn should_replay_task_debug_logs(&self, state: ConcurrentLogState) -> bool {
        self.parallel_task_mode
            && matches!(
                state,
                ConcurrentLogState::Completed | ConcurrentLogState::Failed
            )
            && tracing::enabled!(target: "cpkg::network", tracing::Level::DEBUG)
    }

    fn update_task_state(
        &mut self,
        task_id: usize,
        state: ConcurrentLogState,
        message: Option<String>,
    ) -> Result<()> {
        let replay_debug_logs = self.should_replay_task_debug_logs(state);
        let Some((label, title, command, debug_snapshot)) = ({
            let Some(task) = self.task_mut(task_id) else {
                return Ok(());
            };
            task.set_state(state, message.clone());
            Some((
                task.label.clone(),
                task.title.clone(),
                task.command.clone(),
                replay_debug_logs.then(|| CompletedTaskDebugSnapshot::from(&*task)),
            ))
        }) else {
            return Ok(());
        };
        self.last_updated_task_id = Some(task_id);

        if self.is_tty {
            if debug_snapshot.is_some() {
                self.clear_rendered()?;
                if let Some(snapshot) = &debug_snapshot {
                    emit_task_completion_debug_logs(snapshot);
                }
            }
            self.render()?;
        } else {
            let text = match state {
                ConcurrentLogState::Started | ConcurrentLogState::Completed => title.clone(),
                ConcurrentLogState::Retrying | ConcurrentLogState::Failed => {
                    message.unwrap_or(title)
                }
            };
            write_grouped_event(&label, state, &text)?;
            if matches!(state, ConcurrentLogState::Failed) {
                Term::stderr().write_line(&format!(
                    "  [{}] {}",
                    colorize_prefix(&label),
                    style(format!("$ {}", command)).dim()
                ))?;
            }
            if let Some(snapshot) = &debug_snapshot {
                emit_task_completion_debug_logs(snapshot);
            }
        }

        Ok(())
    }

    fn update_task_state_by_label(
        &mut self,
        label: &str,
        state: ConcurrentLogState,
        message: &str,
    ) -> Result<()> {
        let Some(task_id) = self
            .tasks
            .iter()
            .rev()
            .find(|task| task.label == label)
            .map(|task| task.id)
        else {
            if !self.is_tty {
                write_grouped_event(label, state, message)?;
            }
            return Ok(());
        };

        self.update_task_state(task_id, state, Some(message.to_string()))
    }

    fn finish_success(&mut self) -> Result<()> {
        if self.is_tty {
            self.clear_rendered()?;
        }
        self.reset_session();
        Ok(())
    }

    fn finish_failure(&mut self) {
        self.release_cursor();
        self.rendered_lines = 0;
        self.tasks.clear();
        self.recent_lines.clear();
        self.last_updated_task_id = None;
        self.next_task_id = 0;
        self.parallel_task_mode = false;
    }

    fn ensure_cursor_hidden(&mut self) -> Result<()> {
        if !self.cursor_hidden {
            Term::stderr().hide_cursor()?;
            self.cursor_hidden = true;
        }
        Ok(())
    }

    fn release_cursor(&mut self) {
        if self.cursor_hidden {
            let _ = Term::stderr().show_cursor();
            self.cursor_hidden = false;
        }
    }

    fn clear_rendered(&mut self) -> Result<()> {
        if self.rendered_lines > 0 {
            Term::stderr().clear_last_lines(self.rendered_lines)?;
            self.rendered_lines = 0;
        }
        self.release_cursor();
        Ok(())
    }

    fn reset_session(&mut self) {
        self.tasks.clear();
        self.recent_lines.clear();
        self.last_updated_task_id = None;
        self.next_task_id = 0;
        self.parallel_task_mode = false;
    }

    fn render(&mut self) -> Result<()> {
        if !self.is_tty {
            return Ok(());
        }

        if self.rendered_lines > 0 {
            Term::stderr().clear_last_lines(self.rendered_lines)?;
        }

        let (_, columns) = Term::stderr().size();
        let width = usize::from(columns.saturating_sub(1)).max(20);
        let lines = if self.tasks.len() <= 1 {
            self.render_single_task(width)
        } else {
            self.render_multi_task(width)
        };

        for line in &lines {
            Term::stderr().write_line(line)?;
        }
        Term::stderr().flush()?;
        self.rendered_lines = lines.len();
        Ok(())
    }

    fn render_single_task(&self, width: usize) -> Vec<String> {
        let task = self
            .tasks
            .first()
            .expect("single-task render requires a task");
        let mut lines = Vec::new();
        lines.push(
            style(fit_to_width(&format!("╭─ {}", task.title), width))
                .cyan()
                .bold()
                .to_string(),
        );
        lines.push(fit_to_width(
            &format!("│ Status: {}", task.single_status_line()),
            width,
        ));
        lines.push(fit_to_width(&format!("│ Cmd: {}", task.command), width));
        if task.recent_lines.is_empty() {
            lines.push(fit_to_width("│ Waiting for command output...", width));
        } else {
            for line in &task.recent_lines {
                lines.push(fit_to_width(&format!("│ {}", line), width));
            }
        }
        lines.push(fit_to_width(
            &format!("╰─ Showing last {} log line(s)", task.recent_lines.len()),
            width,
        ));
        lines
    }

    fn render_multi_task(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(
            style(fit_to_width("╭─ Network activity", width))
                .cyan()
                .bold()
                .to_string(),
        );

        let active_tasks = self.tasks.iter().filter(|task| task.is_active());
        let settled_tasks = self.tasks.iter().filter(|task| !task.is_active());
        let ordered_tasks = active_tasks.chain(settled_tasks).collect::<Vec<_>>();
        let hidden_tasks = ordered_tasks.len().saturating_sub(PANEL_MAX_TASK_LINES);

        for task in ordered_tasks.into_iter().skip(hidden_tasks) {
            lines.push(fit_to_width(&format!("│ {}", task.summary_line()), width));
        }
        if hidden_tasks > 0 {
            lines.push(fit_to_width(
                &format!("│ ... {} more task(s)", hidden_tasks),
                width,
            ));
        }

        if let Some(task) = self.last_updated_task() {
            lines.push(fit_to_width(
                &format!("│ Last cmd: [{}] {}", task.label, task.command),
                width,
            ));
        }

        if self.recent_lines.is_empty() {
            lines.push(fit_to_width("│ Waiting for network output...", width));
        } else {
            for line in &self.recent_lines {
                lines.push(fit_to_width(&format!("│ {}", line), width));
            }
        }

        let active_count = self.tasks.iter().filter(|task| task.is_active()).count();
        let completed_count = self
            .tasks
            .iter()
            .filter(|task| task.state == ConcurrentLogState::Completed)
            .count();
        let failed_count = self
            .tasks
            .iter()
            .filter(|task| task.state == ConcurrentLogState::Failed)
            .count();
        let footer = if failed_count > 0 {
            format!(
                "╰─ {} active, {} completed, {} failed",
                active_count, completed_count, failed_count
            )
        } else {
            format!(
                "╰─ {} active, {} completed, showing last {} log line(s)",
                active_count,
                completed_count,
                self.recent_lines.len()
            )
        };
        lines.push(fit_to_width(&footer, width));
        lines
    }

    fn last_updated_task(&self) -> Option<&GroupedTask> {
        self.last_updated_task_id
            .and_then(|task_id| self.tasks.iter().find(|task| task.id == task_id))
    }

    fn task_mut(&mut self, task_id: usize) -> Option<&mut GroupedTask> {
        self.tasks.iter_mut().find(|task| task.id == task_id)
    }
}

impl Drop for GroupedLogState {
    fn drop(&mut self) {
        self.release_cursor();
    }
}

fn write_grouped_event(label: &str, state: ConcurrentLogState, message: &str) -> Result<()> {
    Term::stderr().write_line(&format!(
        "[{}][{}] {}: {}",
        style("network").dim(),
        colorize_prefix(label),
        colorize_state(state),
        message
    ))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct NetworkBatchLogger {
    inner: Arc<Mutex<GroupedLogState>>,
}

impl NetworkBatchLogger {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GroupedLogState::new())),
        }
    }

    pub(super) fn start_task(
        &self,
        label: &str,
        title: &str,
        command: &str,
    ) -> Result<NetworkTaskHandle> {
        let task_id = self.lock_state()?.start_task(label, title, command)?;
        Ok(NetworkTaskHandle {
            logger: self.clone(),
            task_id,
        })
    }

    pub(crate) fn log_retry(&self, label: &str, message: &str) -> Result<()> {
        self.lock_state()?
            .update_task_state_by_label(label, ConcurrentLogState::Retrying, message)
    }

    pub(crate) fn set_parallel_task_mode(&self, enabled: bool) -> Result<()> {
        self.lock_state()?.set_parallel_task_mode(enabled);
        Ok(())
    }

    pub(crate) fn finish_success(&self) -> Result<()> {
        self.lock_state()?.finish_success()
    }

    pub(crate) fn finish_failure(&self) {
        if let Ok(mut state) = self.lock_state() {
            state.finish_failure();
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, GroupedLogState>> {
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("network log lock poisoned"))
    }
}

pub(super) struct NetworkTaskHandle {
    logger: NetworkBatchLogger,
    task_id: usize,
}

impl NetworkTaskHandle {
    pub(super) fn push_line(&self, kind: StreamKind, line: &str) -> Result<()> {
        self.logger
            .lock_state()?
            .push_task_line(self.task_id, kind, line)
    }

    pub(super) fn finish_success(&self) -> Result<()> {
        self.logger.lock_state()?.update_task_state(
            self.task_id,
            ConcurrentLogState::Completed,
            None,
        )
    }

    pub(super) fn finish_failure(&self, message: &str) -> Result<()> {
        self.logger.lock_state()?.update_task_state(
            self.task_id,
            ConcurrentLogState::Failed,
            Some(message.to_string()),
        )
    }
}

pub(super) fn read_stream<R: Read + Send + 'static>(
    reader: R,
    kind: StreamKind,
    sender: mpsc::Sender<StreamEvent>,
) -> thread::JoinHandle<std::io::Result<String>> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut captured = String::new();
        let mut buffer = String::new();

        loop {
            buffer.clear();
            let bytes = reader.read_line(&mut buffer)?;
            if bytes == 0 {
                break;
            }

            captured.push_str(&buffer);
            let line = buffer.trim_end_matches(['\r', '\n']).to_string();
            if !line.is_empty() {
                let _ = sender.send(StreamEvent { kind, line });
            }
        }

        Ok(captured)
    })
}

pub(super) fn join_reader(
    handle: thread::JoinHandle<std::io::Result<String>>,
    stream_name: &str,
) -> Result<String> {
    match handle.join() {
        Ok(result) => result.with_context(|| format!("failed to read {stream_name} stream")),
        Err(_) => anyhow::bail!("{stream_name} reader thread panicked"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompletedTaskDebugSnapshot, GroupedTask, PANEL_MAX_LOG_LINES, StreamKind,
        task_completion_debug_message,
    };
    use crate::project::network::ConcurrentLogState;

    #[test]
    fn recent_log_line_limit_stays_small() {
        let mut lines = Vec::new();
        for index in 0..(PANEL_MAX_LOG_LINES + 2) {
            lines.push(format!("line-{index}"));
        }
        let overflow = lines.len() - PANEL_MAX_LOG_LINES;
        lines.drain(0..overflow);

        assert_eq!(lines.len(), PANEL_MAX_LOG_LINES);
        assert_eq!(lines.first().unwrap(), "line-2");
    }

    #[test]
    fn grouped_task_recent_lines_keep_only_last_entries() {
        let mut task = GroupedTask::new(0, "TrajectoryControl", "pull", "git pull");
        for index in 0..(PANEL_MAX_LOG_LINES + 2) {
            task.push_line(StreamKind::Stderr, &format!("line-{index}"));
        }

        assert_eq!(task.recent_lines.len(), PANEL_MAX_LOG_LINES);
        assert_eq!(task.recent_lines.first().unwrap(), "stderr: line-2");
    }

    #[test]
    fn grouped_task_debug_lines_keep_full_history_for_replay() {
        let mut task = GroupedTask::new(0, "TrajectoryControl", "pull", "git pull");
        for index in 0..(PANEL_MAX_LOG_LINES + 2) {
            task.push_line(StreamKind::Stderr, &format!("line-{index}"));
        }

        let snapshot = CompletedTaskDebugSnapshot::from(&task);

        assert_eq!(snapshot.log_lines.len(), PANEL_MAX_LOG_LINES + 2);
        assert_eq!(snapshot.log_lines.first().unwrap(), "stderr: line-0");
        assert_eq!(snapshot.log_lines.last().unwrap(), "stderr: line-7");
    }

    #[test]
    fn task_completion_debug_message_prefers_status_message_for_failed_task() {
        let mut task = GroupedTask::new(0, "TrajectoryControl", "pull", "git pull");
        task.set_state(
            ConcurrentLogState::Failed,
            Some("pull failed once; retrying".to_string()),
        );

        let snapshot = CompletedTaskDebugSnapshot::from(&task);

        assert_eq!(
            task_completion_debug_message(&snapshot),
            "pull failed once; retrying"
        );
    }

    #[test]
    fn grouped_task_summary_prefers_status_message_for_retry() {
        let mut task = GroupedTask::new(0, "TrajectoryControl", "pull", "git pull");
        task.set_state(
            ConcurrentLogState::Retrying,
            Some("pull failed once; retrying".to_string()),
        );

        assert_eq!(
            task.summary_line(),
            "[TrajectoryControl] retrying: pull failed once; retrying"
        );
    }
}
