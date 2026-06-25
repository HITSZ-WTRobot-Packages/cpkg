use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub trait Scanner {
    fn scan_sources(&self, dir: &Path) -> Vec<String>;
    fn scan_include_dirs(&self, dir: &Path) -> Vec<String>;
}

pub struct DefaultFsScanner {
    ignore_patterns: Vec<String>,
}

impl DefaultFsScanner {
    pub fn new(ignore_patterns: Vec<String>) -> Self {
        Self { ignore_patterns }
    }

    fn is_ignored(&self, path: &str) -> bool {
        self.ignore_patterns.iter().any(|pat| {
            glob::Pattern::new(pat)
                .map(|p| p.matches(path))
                .unwrap_or(false)
        })
    }
}

fn normalize_path(path: &Path) -> Option<String> {
    path.to_str().map(|value| value.replace('\\', "/"))
}

impl Scanner for DefaultFsScanner {
    fn scan_sources(&self, dir: &Path) -> Vec<String> {
        let mut files = vec![];
        self.rec_sources(dir, dir, &mut files);
        files
    }

    fn scan_include_dirs(&self, dir: &Path) -> Vec<String> {
        let mut dirs = HashSet::new();
        self.rec_include_dirs(dir, dir, &mut dirs);
        let mut vec_dirs: Vec<String> = dirs.into_iter().collect();
        vec_dirs.sort();
        vec_dirs
    }
}

impl DefaultFsScanner {
    fn rec_sources(&self, root: &Path, dir: &Path, files: &mut Vec<String>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if dir_name.starts_with('.') {
                        continue;
                    }
                    if let Some(rel) = normalize_path(path.strip_prefix(root).unwrap_or(&path)) {
                        if self.is_ignored(&rel) {
                            continue;
                        }
                    }
                    self.rec_sources(root, &path, files);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "c" || ext == "cpp" {
                        if let Some(s) = normalize_path(&path) {
                            if let Some(rel) =
                                normalize_path(path.strip_prefix(root).unwrap_or(&path))
                            {
                                if self.is_ignored(&rel) {
                                    continue;
                                }
                            }
                            files.push(format!("\"{}\"", s));
                        }
                    }
                }
            }
        }
    }

    fn rec_include_dirs(&self, root: &Path, dir: &Path, dirs: &mut HashSet<String>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if dir_name.starts_with('.') {
                        continue;
                    }
                    if let Some(rel) = normalize_path(path.strip_prefix(root).unwrap_or(&path)) {
                        if self.is_ignored(&rel) {
                            continue;
                        }
                    }
                    self.rec_include_dirs(root, &path, dirs);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "h" || ext == "hpp" {
                        let parent = path.parent().unwrap_or(Path::new("."));
                        let rel_dir = parent.strip_prefix(root).unwrap_or(parent);
                        let dir_str =
                            normalize_path(rel_dir).unwrap_or_else(|| ".".to_string());
                        if self.is_ignored(&dir_str) {
                            continue;
                        }
                        dirs.insert(dir_str);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn normalize_path_uses_forward_slashes() {
        assert_eq!(
            normalize_path(Path::new(r"src\drivers\motor.cpp")).unwrap(),
            "src/drivers/motor.cpp"
        );
        assert_eq!(
            normalize_path(Path::new(r"include\drivers")).unwrap(),
            "include/drivers"
        );
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!(
            "cpkg-scan-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn scan_sources_ignores_matching_glob() {
        let dir = make_temp_dir("ignore-glob");
        fs::create_dir_all(dir.join("build")).unwrap();
        fs::write(dir.join("src.c"), b"").unwrap();
        fs::write(dir.join("build").join("gen.c"), b"").unwrap();

        let scanner = DefaultFsScanner::new(vec!["build/**".to_string()]);
        let sources = scanner.scan_sources(&dir);

        assert!(sources.iter().any(|s| s.contains("src.c")));
        assert!(!sources.iter().any(|s| s.contains("build/gen.c")));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_sources_ignores_by_extension_pattern() {
        let dir = make_temp_dir("ignore-ext");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("main.c"), b"").unwrap();
        fs::write(dir.join("test.c"), b"").unwrap();

        let scanner = DefaultFsScanner::new(vec!["**/test.c".to_string()]);
        let sources = scanner.scan_sources(&dir);

        assert!(sources.iter().any(|s| s.contains("src/main.c")));
        assert!(!sources.iter().any(|s| s.contains("test.c")));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_sources_empty_ignore_patterns_include_all() {
        let dir = make_temp_dir("ignore-empty");
        fs::create_dir_all(dir.join("build")).unwrap();
        fs::write(dir.join("main.c"), b"").unwrap();
        fs::write(dir.join("build").join("gen.c"), b"").unwrap();

        let scanner = DefaultFsScanner::new(vec![]);
        let sources = scanner.scan_sources(&dir);

        assert!(sources.iter().any(|s| s.contains("main.c")));
        assert!(sources.iter().any(|s| s.contains("build/gen.c")));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_include_dirs_ignores_matching_directory() {
        let dir = make_temp_dir("ignore-inc");
        fs::create_dir_all(dir.join("include")).unwrap();
        fs::create_dir_all(dir.join("build").join("include")).unwrap();
        fs::write(dir.join("include").join("api.h"), b"").unwrap();
        fs::write(dir.join("build").join("include").join("gen.h"), b"").unwrap();

        let scanner = DefaultFsScanner::new(vec!["build/**".to_string()]);
        let inc_dirs = scanner.scan_include_dirs(&dir);

        assert!(inc_dirs.contains(&"include".to_string()));
        assert!(!inc_dirs.contains(&"build/include".to_string()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_include_dirs_ignores_matching_file_pattern() {
        let dir = make_temp_dir("ignore-inc-file");
        fs::create_dir_all(dir.join("include")).unwrap();
        fs::write(dir.join("include").join("api.h"), b"").unwrap();
        fs::write(dir.join("include").join("api_test.h"), b"").unwrap();

        let scanner = DefaultFsScanner::new(vec!["**/*_test.h".to_string()]);
        // The include dir itself is not ignored, only the test header file.
        // Since api.h still exists in include/, include/ should still appear.
        let inc_dirs = scanner.scan_include_dirs(&dir);

        assert!(inc_dirs.contains(&"include".to_string()));

        let _ = fs::remove_dir_all(dir);
    }
}
