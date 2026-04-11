use anyhow::{Context, Result};
use console::{Term, style};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;

const PANEL_MAX_LOG_LINES: usize = 6;

pub struct LoggedCommandOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

struct StreamEvent {
    kind: StreamKind,
    line: String,
}

struct TransientLogPanel {
    term: Term,
    title: String,
    command: String,
    status: String,
    recent_lines: Vec<String>,
    rendered_lines: usize,
    is_tty: bool,
    cursor_hidden: bool,
}

impl TransientLogPanel {
    fn new(title: &str, command: &str) -> Result<Self> {
        let term = Term::stderr();
        let is_tty = term.is_term();
        let mut panel = Self {
            term,
            title: title.to_string(),
            command: command.to_string(),
            status: "Running network request...".to_string(),
            recent_lines: Vec::new(),
            rendered_lines: 0,
            is_tty,
            cursor_hidden: false,
        };

        if panel.is_tty {
            panel.term.hide_cursor()?;
            panel.cursor_hidden = true;
            panel.render()?;
        } else {
            panel
                .term
                .write_line(&format!("[network] {}", panel.title))?;
            panel.term.write_line(&format!("  $ {}", panel.command))?;
        }

        Ok(panel)
    }

    fn push_line(&mut self, kind: StreamKind, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }

        let prefix = match kind {
            StreamKind::Stdout => "stdout",
            StreamKind::Stderr => "stderr",
        };
        self.recent_lines.push(format!("{prefix}: {line}"));
        if self.recent_lines.len() > PANEL_MAX_LOG_LINES {
            let overflow = self.recent_lines.len() - PANEL_MAX_LOG_LINES;
            self.recent_lines.drain(0..overflow);
        }

        if self.is_tty {
            self.render()?;
        } else {
            self.term.write_line(&format!("  {prefix}> {line}"))?;
        }

        Ok(())
    }

    fn finish_success(&mut self) -> Result<()> {
        self.status = "Completed.".to_string();
        if self.is_tty {
            self.clear()?;
        } else {
            self.term.write_line("[network] completed")?;
        }
        Ok(())
    }

    fn finish_failure(&mut self, message: &str) -> Result<()> {
        self.status = message.to_string();
        if self.is_tty {
            self.render()?;
        } else {
            self.term
                .write_line(&format!("[network] failed: {message}"))?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        if self.is_tty && self.rendered_lines > 0 {
            self.term.clear_last_lines(self.rendered_lines)?;
            self.rendered_lines = 0;
        }
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        if !self.is_tty {
            return Ok(());
        }

        if self.rendered_lines > 0 {
            self.term.clear_last_lines(self.rendered_lines)?;
        }

        let (_, columns) = self.term.size();
        let width = usize::from(columns.saturating_sub(1)).max(20);

        let mut lines = Vec::new();
        lines.push(
            style(fit_to_width(&format!("╭─ {}", self.title), width))
                .cyan()
                .bold()
                .to_string(),
        );
        lines.push(fit_to_width(&format!("│ Status: {}", self.status), width));
        lines.push(fit_to_width(&format!("│ Cmd: {}", self.command), width));
        if self.recent_lines.is_empty() {
            lines.push(fit_to_width("│ Waiting for command output...", width));
        } else {
            for line in &self.recent_lines {
                lines.push(fit_to_width(&format!("│ {}", line), width));
            }
        }
        lines.push(fit_to_width(
            &format!("╰─ Showing last {} log line(s)", self.recent_lines.len()),
            width,
        ));

        for line in &lines {
            self.term.write_line(line)?;
        }
        self.term.flush()?;
        self.rendered_lines = lines.len();
        Ok(())
    }
}

impl Drop for TransientLogPanel {
    fn drop(&mut self) {
        if self.cursor_hidden {
            let _ = self.term.show_cursor();
        }
    }
}

fn fit_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let text_len = text.chars().count();
    if text_len <= width {
        return text.to_string();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let mut fitted = text.chars().take(width - 3).collect::<String>();
    fitted.push_str("...");
    fitted
}

fn display_argument(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if value.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = value
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '"' | '\'' | '\\'));

    if !needs_quotes {
        return value.into_owned();
    }

    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn format_command(command: &Command) -> String {
    let mut parts = Vec::new();
    parts.push(display_argument(command.get_program()));
    parts.extend(command.get_args().map(display_argument));
    parts.join(" ")
}

fn read_stream<R: Read + Send + 'static>(
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

fn join_reader(
    handle: thread::JoinHandle<std::io::Result<String>>,
    stream_name: &str,
) -> Result<String> {
    match handle.join() {
        Ok(result) => result.with_context(|| format!("failed to read {stream_name} stream")),
        Err(_) => anyhow::bail!("{stream_name} reader thread panicked"),
    }
}

fn exit_status_label(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

fn error_summary(description: &str, status: ExitStatus, stderr: &str, stdout: &str) -> String {
    let details = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .or_else(|| stdout.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| exit_status_label(status));

    format!("{description} failed: {details}")
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

    let (sender, receiver) = mpsc::channel();
    let stdout_handle = read_stream(stdout, StreamKind::Stdout, sender.clone());
    let stderr_handle = read_stream(stderr, StreamKind::Stderr, sender);

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

#[cfg(test)]
mod tests {
    use super::{PANEL_MAX_LOG_LINES, display_argument};

    #[test]
    fn display_argument_quotes_values_with_spaces() {
        assert_eq!(display_argument("plain".as_ref()), "plain");
        assert_eq!(display_argument("with space".as_ref()), "\"with space\"");
    }

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
}
