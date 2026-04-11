use anyhow::{Result, bail};
use console::{Key, Term, style};
use std::collections::BTreeSet;

use super::index::{IndexedPackage, PackageIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPackageGroup {
    pub repo: String,
    pub packages: Vec<PackageChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChoice {
    pub pkgname: String,
    pub path: String,
    pub version: String,
    pub dependencies: Vec<String>,
}

fn package_relative_path(package: &IndexedPackage) -> String {
    let repo_prefix = format!("Modules/{}/", package.repo);
    package
        .path
        .strip_prefix(&repo_prefix)
        .unwrap_or(&package.path)
        .to_string()
}

pub fn group_packages_by_repository(index: &PackageIndex) -> Vec<RepositoryPackageGroup> {
    let repos = index
        .packages
        .iter()
        .map(|package| package.repo.clone())
        .collect::<BTreeSet<_>>();

    repos
        .into_iter()
        .map(|repo| {
            let mut packages = index
                .packages
                .iter()
                .filter(|package| package.repo == repo)
                .map(|package| PackageChoice {
                    pkgname: package.pkgname.clone(),
                    path: package_relative_path(package),
                    version: package.version.clone(),
                    dependencies: package.dependencies.clone(),
                })
                .collect::<Vec<_>>();
            packages.sort_by(|left, right| left.pkgname.cmp(&right.pkgname));
            RepositoryPackageGroup { repo, packages }
        })
        .collect()
}

fn selected_package_indices(
    groups: &[RepositoryPackageGroup],
    initially_selected_packages: &[String],
) -> BTreeSet<(usize, usize)> {
    let selected_names = initially_selected_packages
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut selected_indices = BTreeSet::new();
    for (repo_index, group) in groups.iter().enumerate() {
        for (package_index, package) in group.packages.iter().enumerate() {
            if selected_names.contains(&package.pkgname) {
                selected_indices.insert((repo_index, package_index));
            }
        }
    }
    selected_indices
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

fn fuzzy_score_token(token: &str, candidate: &str) -> Option<i64> {
    let token = token.trim();
    if token.is_empty() {
        return Some(0);
    }

    let token_chars = token
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    if candidate_chars.is_empty() {
        return None;
    }

    let lowered_candidate = candidate_chars
        .iter()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let lowered_candidate_string = lowered_candidate.iter().collect::<String>();
    let lowered_token = token_chars.iter().collect::<String>();

    let mut score = 0i64;
    let mut search_from = 0usize;
    let mut first_match = None;
    let mut previous_match = None;

    for token_char in token_chars {
        let mut found = None;
        for index in search_from..lowered_candidate.len() {
            if lowered_candidate[index] == token_char {
                found = Some(index);
                break;
            }
        }

        let index = found?;
        if first_match.is_none() {
            first_match = Some(index);
        }

        score += 10;
        if index == 0 {
            score += 15;
        } else {
            let previous_character = candidate_chars[index - 1];
            let current_character = candidate_chars[index];
            if !previous_character.is_ascii_alphanumeric() {
                score += 12;
            } else if current_character.is_ascii_uppercase()
                && previous_character.is_ascii_lowercase()
            {
                score += 8;
            }
        }

        if let Some(previous_index) = previous_match {
            if index == previous_index + 1 {
                score += 18;
            } else {
                score -= (index - previous_index - 1) as i64;
            }
        }

        previous_match = Some(index);
        search_from = index + 1;
    }

    if let Some(index) = first_match {
        score -= index as i64;
    }
    score -= (candidate_chars
        .len()
        .saturating_sub(lowered_token.chars().count())) as i64;
    if lowered_candidate_string.contains(&lowered_token) {
        score += 30;
    }

    Some(score)
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let tokens = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Some(0);
    }

    let mut total = 0i64;
    for token in tokens {
        total += fuzzy_score_token(token, candidate)?;
    }
    Some(total)
}

fn package_search_score(query: &str, repository: &str, package: &PackageChoice) -> Option<i64> {
    let dependency_text = package.dependencies.join(" ");
    [
        fuzzy_score(query, &package.pkgname).map(|score| score + 60),
        fuzzy_score(query, &package.path).map(|score| score + 30),
        fuzzy_score(query, &dependency_text).map(|score| score + 15),
        fuzzy_score(
            query,
            &format!("{} {} {}", package.pkgname, package.path, dependency_text),
        )
        .map(|score| score + 5),
        fuzzy_score(query, repository),
    ]
    .into_iter()
    .flatten()
    .max()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisibleItem {
    Repository(usize),
    Package {
        repo_index: usize,
        package_index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Navigate,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilteredRepository {
    repo_index: usize,
    package_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct TreePickerState {
    groups: Vec<RepositoryPackageGroup>,
    expanded_repositories: BTreeSet<usize>,
    selected_packages: BTreeSet<(usize, usize)>,
    cursor: usize,
    scroll_offset: usize,
    search_query: String,
    input_mode: InputMode,
}

impl TreePickerState {
    fn new(groups: Vec<RepositoryPackageGroup>, initially_selected_packages: &[String]) -> Self {
        let selected_packages = selected_package_indices(&groups, initially_selected_packages);
        Self {
            groups,
            expanded_repositories: BTreeSet::new(),
            selected_packages,
            cursor: 0,
            scroll_offset: 0,
            search_query: String::new(),
            input_mode: InputMode::Navigate,
        }
    }

    fn search_active(&self) -> bool {
        !self.search_query.trim().is_empty()
    }

    fn filtered_repositories(&self) -> Vec<FilteredRepository> {
        let query = self.search_query.trim();
        if query.is_empty() {
            return self
                .groups
                .iter()
                .enumerate()
                .map(|(repo_index, group)| FilteredRepository {
                    repo_index,
                    package_indices: (0..group.packages.len()).collect(),
                })
                .collect();
        }

        let mut filtered = Vec::new();
        for (repo_index, group) in self.groups.iter().enumerate() {
            let repository_match = fuzzy_score(query, &group.repo);
            let matching_packages = group
                .packages
                .iter()
                .enumerate()
                .filter_map(|(package_index, package)| {
                    package_search_score(query, &group.repo, package).map(|_| package_index)
                })
                .collect::<Vec<_>>();

            if repository_match.is_some() || !matching_packages.is_empty() {
                let package_indices = if repository_match.is_some() {
                    (0..group.packages.len()).collect()
                } else {
                    matching_packages
                };
                filtered.push(FilteredRepository {
                    repo_index,
                    package_indices,
                });
            }
        }

        filtered
    }

    fn visible_items(&self) -> Vec<VisibleItem> {
        let mut items = Vec::new();

        if self.search_active() {
            for filtered_repository in self.filtered_repositories() {
                items.push(VisibleItem::Repository(filtered_repository.repo_index));
                for package_index in filtered_repository.package_indices {
                    items.push(VisibleItem::Package {
                        repo_index: filtered_repository.repo_index,
                        package_index,
                    });
                }
            }
            return items;
        }

        for (repo_index, group) in self.groups.iter().enumerate() {
            items.push(VisibleItem::Repository(repo_index));
            if self.expanded_repositories.contains(&repo_index) {
                for package_index in 0..group.packages.len() {
                    items.push(VisibleItem::Package {
                        repo_index,
                        package_index,
                    });
                }
            }
        }

        items
    }

    fn current_item(&self) -> Option<VisibleItem> {
        self.visible_items().get(self.cursor).copied()
    }

    fn reset_cursor_for_filter_change(&mut self) {
        self.cursor = 0;
        self.scroll_offset = 0;
        self.ensure_cursor_valid();
    }

    fn ensure_cursor_valid(&mut self) {
        let len = self.visible_items().len();
        if len == 0 {
            self.cursor = 0;
            self.scroll_offset = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self) {
        let len = self.visible_items().len();
        if self.cursor + 1 < len {
            self.cursor += 1;
        }
    }

    fn toggle_current(&mut self) {
        match self.current_item() {
            Some(VisibleItem::Repository(repo_index)) => self.toggle_repository(repo_index),
            Some(VisibleItem::Package {
                repo_index,
                package_index,
            }) => self.toggle_package(repo_index, package_index),
            None => {}
        }
    }

    fn expand_current(&mut self) {
        if self.search_active() {
            return;
        }

        if let Some(VisibleItem::Repository(repo_index)) = self.current_item() {
            self.expanded_repositories.insert(repo_index);
        }
    }

    fn collapse_current(&mut self) {
        match self.current_item() {
            Some(VisibleItem::Repository(repo_index)) => {
                if self.search_active() {
                    return;
                }
                self.expanded_repositories.remove(&repo_index);
                self.ensure_cursor_valid();
            }
            Some(VisibleItem::Package { repo_index, .. }) => {
                self.move_cursor_to_repository(repo_index);
            }
            None => {}
        }
    }

    fn toggle_repository(&mut self, repo_index: usize) {
        if self.search_active() {
            return;
        }

        if !self.expanded_repositories.insert(repo_index) {
            self.expanded_repositories.remove(&repo_index);
        }
        self.ensure_cursor_valid();
    }

    fn toggle_package(&mut self, repo_index: usize, package_index: usize) {
        let key = (repo_index, package_index);
        if !self.selected_packages.insert(key) {
            self.selected_packages.remove(&key);
        }
    }

    fn move_cursor_to_repository(&mut self, repo_index: usize) {
        if let Some(position) = self
            .visible_items()
            .iter()
            .position(|item| *item == VisibleItem::Repository(repo_index))
        {
            self.cursor = position;
        }
    }

    fn ensure_cursor_visible(&mut self, viewport_rows: usize) {
        if viewport_rows == 0 {
            self.scroll_offset = 0;
            return;
        }

        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + viewport_rows {
            self.scroll_offset = self.cursor + 1 - viewport_rows;
        }
    }

    fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::Search;
    }

    fn exit_search_mode(&mut self) {
        self.input_mode = InputMode::Navigate;
    }

    fn clear_search(&mut self) {
        self.search_query.clear();
        self.exit_search_mode();
        self.reset_cursor_for_filter_change();
    }

    fn push_search_char(&mut self, character: char) {
        self.search_query.push(character);
        self.reset_cursor_for_filter_change();
    }

    fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.reset_cursor_for_filter_change();
    }

    fn selected_package_names(&self) -> Vec<String> {
        let mut packages = Vec::new();
        for (repo_index, group) in self.groups.iter().enumerate() {
            for (package_index, package) in group.packages.iter().enumerate() {
                if self
                    .selected_packages
                    .contains(&(repo_index, package_index))
                {
                    packages.push(package.pkgname.clone());
                }
            }
        }
        packages
    }

    fn selected_count_for_repository(&self, repo_index: usize) -> usize {
        self.groups[repo_index]
            .packages
            .iter()
            .enumerate()
            .filter(|(package_index, _)| {
                self.selected_packages
                    .contains(&(repo_index, *package_index))
            })
            .count()
    }

    fn search_status_line(&self) -> String {
        if self.input_mode == InputMode::Search {
            format!(
                "Search: {} [editing; Enter finish, Backspace delete, Esc clear]",
                self.search_query
            )
        } else if self.search_active() {
            format!("Search: {} [active; / edit, Esc clear]", self.search_query)
        } else {
            "Search: off [/ to start fuzzy search]".to_string()
        }
    }

    fn action_status_line(&self) -> &'static str {
        if self.input_mode == InputMode::Search {
            "Up/Down move | Type to filter | Enter finish search"
        } else if self.search_active() {
            "Up/Down move | Space toggle package | Left parent | / edit search"
        } else {
            "Up/Down move | Space toggle | Right expand | Left collapse | / search"
        }
    }

    fn confirm_status_line(&self) -> &'static str {
        if self.input_mode == InputMode::Search {
            "Esc clear search | q types literal q while editing"
        } else if self.search_active() {
            "Enter confirm | Esc clear search | q cancel"
        } else {
            "Enter confirm | Esc/q cancel"
        }
    }
}

struct CursorGuard<'a>(&'a Term);

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

fn render_item(state: &TreePickerState, item: VisibleItem) -> String {
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

fn viewport_rows(rows: u16, header_lines: usize, footer_lines: usize) -> usize {
    usize::from(rows)
        .saturating_sub(header_lines + footer_lines + 1)
        .max(1)
}

fn render_picker(
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
                Key::Char('/') if state.input_mode == InputMode::Navigate => {
                    state.enter_search_mode();
                }
                Key::Backspace if state.input_mode == InputMode::Search => {
                    state.pop_search_char();
                }
                Key::Enter if state.input_mode == InputMode::Search => {
                    state.exit_search_mode();
                }
                Key::Escape if state.input_mode == InputMode::Search => {
                    if state.search_active() {
                        state.clear_search();
                    } else {
                        state.exit_search_mode();
                    }
                }
                Key::Char(character) if state.input_mode == InputMode::Search => {
                    state.push_search_char(character);
                }
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

#[cfg(test)]
mod tests {
    use super::{
        TreePickerState, VisibleItem, fuzzy_score, group_packages_by_repository,
        selected_package_indices, viewport_rows,
    };
    use crate::project::index::{IndexedPackage, PackageIndex};

    fn sample_index() -> PackageIndex {
        PackageIndex {
            generated_at: None,
            packages: vec![
                IndexedPackage {
                    repo: "MotorDrivers".to_string(),
                    path: "Modules/MotorDrivers/motors/DJI".to_string(),
                    name: "DJI".to_string(),
                    pkgname: "MotorDrivers::DJI".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["bsp::CANDriver".to_string()],
                },
                IndexedPackage {
                    repo: "BasicComponents".to_string(),
                    path: "Modules/BasicComponents/bsp/can_driver".to_string(),
                    name: "CANDriver".to_string(),
                    pkgname: "bsp::CANDriver".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: vec!["stm32cubemx".to_string()],
                },
                IndexedPackage {
                    repo: "MotorDrivers".to_string(),
                    path: "Modules/MotorDrivers/core".to_string(),
                    name: "Core".to_string(),
                    pkgname: "MotorDrivers::Core".to_string(),
                    version: "0.1.0".to_string(),
                    dependencies: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn group_packages_by_repository_sorts_repositories_and_packages() {
        let groups = group_packages_by_repository(&sample_index());

        assert_eq!(groups[0].repo, "BasicComponents");
        assert_eq!(groups[0].packages[0].pkgname, "bsp::CANDriver");
        assert_eq!(groups[0].packages[0].path, "bsp/can_driver");
        assert_eq!(groups[0].packages[0].version, "0.1.0");
        assert_eq!(groups[1].repo, "MotorDrivers");
        assert_eq!(groups[1].packages[0].pkgname, "MotorDrivers::Core");
        assert_eq!(groups[1].packages[1].pkgname, "MotorDrivers::DJI");
    }

    #[test]
    fn fuzzy_score_matches_subsequence_tokens() {
        assert!(fuzzy_score("can drv", "CANDriver").is_some());
        assert!(fuzzy_score("mdi", "MotorDrivers::DJI").is_some());
        assert!(fuzzy_score("xyz", "MotorDrivers::DJI").is_none());
    }

    #[test]
    fn tree_picker_expands_repository_inline() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        assert_eq!(
            state.visible_items(),
            vec![VisibleItem::Repository(0), VisibleItem::Repository(1)]
        );

        state.move_down();
        state.expand_current();

        assert_eq!(
            state.visible_items(),
            vec![
                VisibleItem::Repository(0),
                VisibleItem::Repository(1),
                VisibleItem::Package {
                    repo_index: 1,
                    package_index: 0
                },
                VisibleItem::Package {
                    repo_index: 1,
                    package_index: 1
                }
            ]
        );
    }

    #[test]
    fn tree_picker_selects_packages_in_display_order() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        state.move_down();
        state.expand_current();
        state.move_down();
        state.toggle_current();
        state.move_down();
        state.toggle_current();

        assert_eq!(
            state.selected_package_names(),
            vec!["MotorDrivers::Core", "MotorDrivers::DJI"]
        );
    }

    #[test]
    fn tree_picker_left_on_package_moves_to_parent_repository() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        state.move_down();
        state.expand_current();
        state.move_down();
        state.collapse_current();

        assert_eq!(state.current_item(), Some(VisibleItem::Repository(1)));
    }

    #[test]
    fn tree_picker_search_filters_to_matching_package() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        state.search_query = "dji".to_string();
        state.reset_cursor_for_filter_change();

        assert_eq!(
            state.visible_items(),
            vec![
                VisibleItem::Repository(1),
                VisibleItem::Package {
                    repo_index: 1,
                    package_index: 1
                }
            ]
        );
    }

    #[test]
    fn tree_picker_search_on_repository_shows_all_child_packages() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        state.search_query = "motor".to_string();
        state.reset_cursor_for_filter_change();

        assert_eq!(
            state.visible_items(),
            vec![
                VisibleItem::Repository(1),
                VisibleItem::Package {
                    repo_index: 1,
                    package_index: 0
                },
                VisibleItem::Package {
                    repo_index: 1,
                    package_index: 1
                }
            ]
        );
    }

    #[test]
    fn tree_picker_search_preserves_selected_packages() {
        let groups = group_packages_by_repository(&sample_index());
        let mut state = TreePickerState::new(groups, &[]);

        state.move_down();
        state.expand_current();
        state.move_down();
        state.toggle_current();

        state.search_query = "dji".to_string();
        state.reset_cursor_for_filter_change();
        assert_eq!(state.selected_package_names(), vec!["MotorDrivers::Core"]);

        state.clear_search();
        assert_eq!(state.selected_package_names(), vec!["MotorDrivers::Core"]);
    }

    #[test]
    fn selected_package_indices_marks_initially_selected_dependencies() {
        let groups = group_packages_by_repository(&sample_index());

        let selected = selected_package_indices(
            &groups,
            &[
                "MotorDrivers::DJI".to_string(),
                "missing::Package".to_string(),
            ],
        );

        assert!(selected.contains(&(1, 1)));
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn tree_picker_starts_with_initial_selection_checked() {
        let groups = group_packages_by_repository(&sample_index());
        let state = TreePickerState::new(groups, &["MotorDrivers::DJI".to_string()]);

        assert_eq!(state.selected_package_names(), vec!["MotorDrivers::DJI"]);
    }

    #[test]
    fn viewport_rows_reserves_one_terminal_row_for_redraw() {
        assert_eq!(viewport_rows(24, 5, 1), 17);
        assert_eq!(viewport_rows(10, 5, 1), 3);
    }
}
