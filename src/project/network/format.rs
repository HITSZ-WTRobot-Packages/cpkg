use console::{measure_text_width, strip_ansi_codes, style, truncate_str};
use std::ffi::OsStr;
use std::process::{Command, ExitStatus};

use super::ConcurrentLogState;

const ELLIPSIS: &str = "...";

pub(super) fn fit_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    if measure_text_width(text) <= width {
        return text.to_string();
    }

    if width <= ELLIPSIS.len() {
        return ".".repeat(width);
    }

    truncate_str(text, width, ELLIPSIS).into_owned()
}

/// Reduce one raw stream chunk to the single screen row a terminal would show:
/// keep the text after the last carriage return (git progress redraws), drop ANSI
/// escapes, and normalize tabs so row accounting stays exact.
pub(super) fn sanitize_stream_line(raw: &str) -> String {
    let visible = raw
        .trim_end_matches(['\r', '\n'])
        .rsplit('\r')
        .next()
        .unwrap_or("");

    strip_ansi_codes(visible)
        .chars()
        .map(|character| if character == '\t' { ' ' } else { character })
        .filter(|character| !character.is_control())
        .collect()
}

pub(super) fn display_argument(value: &OsStr) -> String {
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

pub(super) fn format_command(command: &Command) -> String {
    let mut parts = Vec::new();
    parts.push(display_argument(command.get_program()));
    parts.extend(command.get_args().map(display_argument));
    parts.join(" ")
}

fn exit_status_label(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

pub(super) fn error_summary(
    description: &str,
    status: ExitStatus,
    stderr: &str,
    stdout: &str,
) -> String {
    let details = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .or_else(|| stdout.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| exit_status_label(status));

    format!("{description} failed: {details}")
}

pub(super) fn colorize_prefix(prefix: &str) -> String {
    let palette_slot = prefix
        .bytes()
        .fold(0u8, |accumulator, byte| accumulator.wrapping_add(byte))
        % 6;

    let styled = match palette_slot {
        0 => style(prefix).blue(),
        1 => style(prefix).cyan(),
        2 => style(prefix).green(),
        3 => style(prefix).yellow(),
        4 => style(prefix).magenta(),
        _ => style(prefix).white(),
    };

    styled.bold().to_string()
}

pub(super) fn state_label(state: ConcurrentLogState) -> &'static str {
    match state {
        ConcurrentLogState::Started => "started",
        ConcurrentLogState::Completed => "completed",
        ConcurrentLogState::Failed => "failed",
        ConcurrentLogState::Retrying => "retrying",
    }
}

pub(super) fn colorize_state(state: ConcurrentLogState) -> String {
    let base = state_label(state);

    match state {
        ConcurrentLogState::Started | ConcurrentLogState::Retrying => {
            style(base).yellow().bold().to_string()
        }
        ConcurrentLogState::Completed => style(base).green().bold().to_string(),
        ConcurrentLogState::Failed => style(base).red().bold().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        colorize_state, display_argument, error_summary, fit_to_width, sanitize_stream_line,
    };
    use crate::project::network::ConcurrentLogState;
    use console::measure_text_width;
    use std::process::Command;

    #[test]
    fn fit_to_width_measures_display_width() {
        let width = 20;
        let text = "子模组路径 'Modules/BasicComponents'：检出 'abc'";

        let fitted = fit_to_width(text, width);

        assert!(measure_text_width(&fitted) <= width);
        assert!(fitted.ends_with("..."));
        assert_eq!(fit_to_width("short", 20), "short");
        assert_eq!(fit_to_width("abc", 0), "");
    }

    #[test]
    fn sanitize_stream_line_keeps_last_progress_segment() {
        assert_eq!(
            sanitize_stream_line("Receiving objects:  45%\rReceiving objects: 100%\n"),
            "Receiving objects: 100%"
        );
    }

    #[test]
    fn sanitize_stream_line_strips_ansi_and_tabs() {
        assert_eq!(sanitize_stream_line("\u{1b}[32mok\u{1b}[0m\ta\n"), "ok a");
    }

    #[test]
    fn display_argument_quotes_values_with_spaces() {
        assert_eq!(display_argument("plain".as_ref()), "plain");
        assert_eq!(display_argument("with space".as_ref()), "\"with space\"");
    }

    #[test]
    fn error_summary_prefers_first_non_empty_stderr_line() {
        let status = Command::new("git")
            .args(["rev-parse", "--definitely-invalid-option"])
            .output()
            .unwrap()
            .status;

        let summary = error_summary(
            "pulling latest main",
            status,
            "\nremote rejected update\nextra detail\n",
            "stdout detail\n",
        );

        assert_eq!(
            summary,
            "pulling latest main failed: remote rejected update"
        );
    }

    #[test]
    fn colorize_state_uses_distinct_labels() {
        let started = colorize_state(ConcurrentLogState::Started);
        let completed = colorize_state(ConcurrentLogState::Completed);
        let failed = colorize_state(ConcurrentLogState::Failed);
        let retrying = colorize_state(ConcurrentLogState::Retrying);

        assert!(started.contains("started"));
        assert!(completed.contains("completed"));
        assert!(failed.contains("failed"));
        assert!(retrying.contains("retrying"));
    }
}
