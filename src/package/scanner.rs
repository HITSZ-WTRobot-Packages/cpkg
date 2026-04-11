use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub trait Scanner {
    fn scan_sources(&self, dir: &Path) -> Vec<String>;
    fn scan_include_dirs(&self, dir: &Path) -> Vec<String>;
}

#[derive(Default)]
pub struct DefaultFsScanner;

impl Scanner for DefaultFsScanner {
    fn scan_sources(&self, dir: &Path) -> Vec<String> {
        fn rec(dir: &Path, files: &mut Vec<String>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with('.') {
                                continue;
                            }
                        }
                        rec(&path, files);
                    } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext == "c" || ext == "cpp" {
                            if let Some(s) = path.to_str() {
                                files.push(format!("\"{}\"", s));
                            }
                        }
                    }
                }
            }
        }
        let mut files = vec![];
        rec(dir, &mut files);
        files
    }

    fn scan_include_dirs(&self, dir: &Path) -> Vec<String> {
        let mut dirs = HashSet::new();
        fn rec(dir: &Path, dirs: &mut HashSet<String>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with('.') {
                                continue;
                            }
                        }
                        rec(&path, dirs);
                    } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext == "h" || ext == "hpp" {
                            let dir_str = path
                                .parent()
                                .unwrap_or(Path::new("."))
                                .to_str()
                                .unwrap_or(".");
                            dirs.insert(dir_str.replace("\\", "/"));
                        }
                    }
                }
            }
        }
        rec(dir, &mut dirs);
        let mut vec_dirs: Vec<String> = dirs.into_iter().collect();
        vec_dirs.sort();
        vec_dirs
    }
}
