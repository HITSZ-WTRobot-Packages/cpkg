use anyhow::Result;
use console::{Term, style};

use super::model::PackageChoice;
use super::state::{TreePickerState, VisibleItem};

pub(super) struct CursorGuard<'a>(pub(super) &'a Term);

impl Drop for CursorGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.show_cursor();
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

fn package_label(package: &PackageChoice) -> String {
    let mut details = Vec::new();
    if !package.version.is_empty() {
        details.push(format!("v{}", package.version));
    }
    details.push(package.path.clone());
    if !package.dependencies.is_empty() {
        details.push(format!("deps: {}", package.dependencies.join(", ")));
    }

    format!("{}  [{}]", package.pkgname, details.join(" | "))
}

fn render_item(state: &TreePickerState, item: VisibleItem) -> String {
    // TODO: Add character-level search-hit highlighting when the user explicitly requests it.
    match item {
        VisibleItem::Repository(repo_index) => {
            let group = &state.groups[repo_index];
            let marker = if state.search_active() {
                "[~]"
            } else if state.expanded_repositories.contains(&repo_index) {
                "[-]"
            } else {
                "[+]"
            };
            let selected = state.selected_count_for_repository(repo_index);
            format!(
                "{} {} ({} selected / {} total)",
                marker,
                group.repo,
                selected,
                group.packages.len()
            )
        }
        VisibleItem::Package {
            repo_index,
            package_index,
        } => {
            let package = &state.groups[repo_index].packages[package_index];
            let checkbox = if state
                .selected_packages
                .contains(&(repo_index, package_index))
            {
                "[x]"
            } else {
                "[ ]"
            };
            format!("  {} {}", checkbox, package_label(package))
        }
    }
}

fn item_is_selected(state: &TreePickerState, item: VisibleItem) -> bool {
    match item {
        VisibleItem::Repository(repo_index) => state.selected_count_for_repository(repo_index) > 0,
        VisibleItem::Package {
            repo_index,
            package_index,
        } => state
            .selected_packages
            .contains(&(repo_index, package_index)),
    }
}

pub(super) fn viewport_rows(rows: u16, header_lines: usize, footer_lines: usize) -> usize {
    usize::from(rows)
        .saturating_sub(header_lines + footer_lines + 1)
        .max(1)
}

pub(super) fn render_picker(
    term: &Term,
    state: &mut TreePickerState,
    previous_line_count: usize,
) -> Result<usize> {
    if previous_line_count > 0 {
        term.clear_last_lines(previous_line_count)?;
    }

    let visible_items = state.visible_items();
    let (rows, columns) = term.size();
    let width = usize::from(columns.saturating_sub(1)).max(1);
    let header_lines = 5usize;
    let footer_lines = 1usize;
    let viewport_rows = viewport_rows(rows, header_lines, footer_lines);
    state.ensure_cursor_visible(viewport_rows);

    let start = state
        .scroll_offset
        .min(visible_items.len().saturating_sub(1));
    let end = (start + viewport_rows).min(visible_items.len());
    let total_selected = state.selected_packages.len();

    let mut lines = Vec::new();
    lines.push(
        style(fit_to_width(
            "Select driver packages to add to wtrproject.toml.",
            width,
        ))
        .cyan()
        .bold()
        .to_string(),
    );
    lines.push(fit_to_width(state.action_status_line(), width));
    lines.push(fit_to_width(&state.search_status_line(), width));
    lines.push(fit_to_width(
        &format!("Selected {} package(s)", total_selected),
        width,
    ));
    lines.push(fit_to_width(state.confirm_status_line(), width));

    if visible_items.is_empty() {
        lines.push(fit_to_width("No matching repositories or packages.", width));
    } else {
        for (offset, item) in visible_items[start..end].iter().enumerate() {
            let absolute_index = start + offset;
            let line = fit_to_width(&render_item(state, *item), width);
            let is_cursor = absolute_index == state.cursor;
            let is_selected = item_is_selected(state, *item);
            let styled_line = match (is_cursor, is_selected) {
                (true, true) => style(line).green().bold().reverse().to_string(),
                (true, false) => style(line).reverse().to_string(),
                (false, true) => style(line).green().bold().to_string(),
                (false, false) => line,
            };
            lines.push(styled_line);
        }
    }

    if visible_items.is_empty() {
        lines.push(fit_to_width("Items 0-0 of 0", width));
    } else {
        lines.push(fit_to_width(
            &format!(
                "Items {}-{} of {}",
                start + 1,
                end.max(start + 1),
                visible_items.len()
            ),
            width,
        ));
    }

    for line in &lines {
        term.write_line(line)?;
    }
    term.flush()?;

    Ok(lines.len())
}

#[cfg(test)]
mod tests {
    use super::viewport_rows;

    #[test]
    fn viewport_rows_reserves_one_terminal_row_for_redraw() {
        assert_eq!(viewport_rows(24, 5, 1), 17);
        assert_eq!(viewport_rows(10, 5, 1), 3);
    }
}
