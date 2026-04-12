use std::collections::BTreeSet;

use super::model::{RepositoryPackageGroup, selected_package_indices};
use super::search::{fuzzy_score, package_search_score};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VisibleItem {
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
pub(super) struct TreePickerState {
    pub(super) groups: Vec<RepositoryPackageGroup>,
    pub(super) expanded_repositories: BTreeSet<usize>,
    pub(super) selected_packages: BTreeSet<(usize, usize)>,
    pub(super) cursor: usize,
    pub(super) scroll_offset: usize,
    pub(super) search_query: String,
    input_mode: InputMode,
}

impl TreePickerState {
    pub(super) fn new(
        groups: Vec<RepositoryPackageGroup>,
        initially_selected_packages: &[String],
    ) -> Self {
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

    pub(super) fn is_search_mode(&self) -> bool {
        self.input_mode == InputMode::Search
    }

    pub(super) fn search_active(&self) -> bool {
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

    pub(super) fn visible_items(&self) -> Vec<VisibleItem> {
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

    pub(super) fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub(super) fn move_down(&mut self) {
        let len = self.visible_items().len();
        if self.cursor + 1 < len {
            self.cursor += 1;
        }
    }

    pub(super) fn toggle_current(&mut self) {
        match self.current_item() {
            Some(VisibleItem::Repository(repo_index)) => self.toggle_repository(repo_index),
            Some(VisibleItem::Package {
                repo_index,
                package_index,
            }) => self.toggle_package(repo_index, package_index),
            None => {}
        }
    }

    pub(super) fn expand_current(&mut self) {
        if self.search_active() {
            return;
        }

        if let Some(VisibleItem::Repository(repo_index)) = self.current_item() {
            self.expanded_repositories.insert(repo_index);
        }
    }

    pub(super) fn collapse_current(&mut self) {
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

    pub(super) fn ensure_cursor_visible(&mut self, viewport_rows: usize) {
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

    pub(super) fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::Search;
    }

    pub(super) fn exit_search_mode(&mut self) {
        self.input_mode = InputMode::Navigate;
    }

    pub(super) fn clear_search(&mut self) {
        self.search_query.clear();
        self.exit_search_mode();
        self.reset_cursor_for_filter_change();
    }

    pub(super) fn push_search_char(&mut self, character: char) {
        self.search_query.push(character);
        self.reset_cursor_for_filter_change();
    }

    pub(super) fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.reset_cursor_for_filter_change();
    }

    pub(super) fn selected_package_names(&self) -> Vec<String> {
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

    pub(super) fn selected_count_for_repository(&self, repo_index: usize) -> usize {
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

    pub(super) fn search_status_line(&self) -> String {
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

    pub(super) fn action_status_line(&self) -> &'static str {
        if self.input_mode == InputMode::Search {
            "Up/Down move | Type to filter | Enter finish search"
        } else if self.search_active() {
            "Up/Down move | Space toggle package | Left parent | / edit search"
        } else {
            "Up/Down move | Space toggle | Right expand | Left collapse | / search"
        }
    }

    pub(super) fn confirm_status_line(&self) -> &'static str {
        if self.input_mode == InputMode::Search {
            "Esc clear search | q types literal q while editing"
        } else if self.search_active() {
            "Enter confirm | Esc clear search | q cancel"
        } else {
            "Enter confirm | Esc/q cancel"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TreePickerState, VisibleItem};
    use crate::project::index::{IndexedPackage, PackageIndex};
    use crate::project::interactive::group_packages_by_repository;

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
    fn tree_picker_starts_with_initial_selection_checked() {
        let groups = group_packages_by_repository(&sample_index());
        let state = TreePickerState::new(groups, &["MotorDrivers::DJI".to_string()]);

        assert_eq!(state.selected_package_names(), vec!["MotorDrivers::DJI"]);
    }
}
