mod model;
mod render;
mod search;
mod state;

use anyhow::{Result, bail};
use console::{Key, Term};

pub use self::model::{PackageChoice, RepositoryPackageGroup, group_packages_by_repository};
use self::render::{CursorGuard, render_picker};
use self::state::TreePickerState;
use crate::project::index::PackageIndex;

pub fn select_dependencies(index: &PackageIndex) -> Result<Option<Vec<String>>> {
    select_dependencies_with_initial_selection(index, &[])
}

pub fn select_dependencies_with_initial_selection(
    index: &PackageIndex,
    initially_selected_packages: &[String],
) -> Result<Option<Vec<String>>> {
    let groups = group_packages_by_repository(index);
    if groups.is_empty() {
        Term::stderr().write_line("No packages found in the package index.")?;
        return Ok(Some(Vec::new()));
    }

    let term = Term::stderr();
    if !term.is_term() {
        bail!("interactive mode requires an attached terminal");
    }

    let mut state = TreePickerState::new(groups, initially_selected_packages);
    let mut rendered_line_count = 0usize;
    term.hide_cursor()?;
    let _cursor_guard = CursorGuard(&term);

    let interaction_result = (|| -> Result<Option<Vec<String>>> {
        loop {
            rendered_line_count = render_picker(&term, &mut state, rendered_line_count)?;

            match term.read_key()? {
                Key::Char('/') if !state.is_search_mode() => state.enter_search_mode(),
                Key::Backspace if state.is_search_mode() => state.pop_search_char(),
                Key::Enter if state.is_search_mode() => state.exit_search_mode(),
                Key::Escape if state.is_search_mode() => {
                    if state.search_active() {
                        state.clear_search();
                    } else {
                        state.exit_search_mode();
                    }
                }
                Key::Char(character) if state.is_search_mode() => state.push_search_char(character),
                Key::ArrowUp | Key::Char('k') => state.move_up(),
                Key::ArrowDown | Key::Char('j') => state.move_down(),
                Key::ArrowRight | Key::Char('l') => state.expand_current(),
                Key::ArrowLeft | Key::Char('h') => state.collapse_current(),
                Key::Char(' ') => state.toggle_current(),
                Key::Enter => return Ok(Some(state.selected_package_names())),
                Key::Escape if state.search_active() => state.clear_search(),
                Key::Escape | Key::Char('q') => return Ok(None),
                _ => {}
            }
        }
    })();

    if rendered_line_count > 0 {
        term.clear_last_lines(rendered_line_count)?;
    }

    match interaction_result? {
        Some(selected_packages) => Ok(Some(selected_packages)),
        None => {
            term.write_line("Selection cancelled.")?;
            Ok(None)
        }
    }
}
