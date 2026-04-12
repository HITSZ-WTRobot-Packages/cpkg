use anyhow::{Context, Result};
use console::{Term, style};
use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc;
use std::thread;

use super::format::fit_to_width;

pub(super) const PANEL_MAX_LOG_LINES: usize = 6;

#[derive(Clone, Copy)]
pub(super) enum StreamKind {
    Stdout,
    Stderr,
}

pub(super) struct StreamEvent {
    pub(super) kind: StreamKind,
    pub(super) line: String,
}

pub(super) struct TransientLogPanel {
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
    pub(super) fn new(title: &str, command: &str) -> Result<Self> {
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

    pub(super) fn push_line(&mut self, kind: StreamKind, line: &str) -> Result<()> {
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

    pub(super) fn finish_success(&mut self) -> Result<()> {
        self.status = "Completed.".to_string();
        if self.is_tty {
            self.clear()?;
        } else {
            self.term.write_line("[network] completed")?;
        }
        Ok(())
    }

    pub(super) fn finish_failure(&mut self, message: &str) -> Result<()> {
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
    use super::PANEL_MAX_LOG_LINES;

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
