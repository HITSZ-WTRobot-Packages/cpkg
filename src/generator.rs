use crate::Cpkg;
use crate::scanner::Scanner;
use std::fs;
use std::path::Path;

pub trait Generator {
    /// Generate content (as string) for given `Cpkg`, using provided `Scanner` to discover files.
    fn generate_string(&self, cpkg: &Cpkg, scanner: &dyn Scanner) -> String;
    /// Write the generated content to a target path (like CMakeLists.txt).
    fn write_to(&self, cpkg: &Cpkg, scanner: &dyn Scanner, target: &Path) -> std::io::Result<()> {
        let content = self.generate_string(cpkg, scanner);
        fs::write(target, content)
    }
}

pub struct CMakeGenerator;

impl CMakeGenerator {
    fn pascal_case(s: &str) -> String {
        s.split(|c| c == '_' || c == ' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<String>()
    }
}

impl Default for CMakeGenerator {
    fn default() -> Self {
        CMakeGenerator
    }
}

impl Generator for CMakeGenerator {
    fn generate_string(&self, cpkg: &Cpkg, scanner: &dyn Scanner) -> String {
        let parts: Vec<&str> = cpkg.pkgname.split("::").collect();
        let (namespace, name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            ("", cpkg.pkgname.as_str())
        };
        let alias_name = cpkg.pkgname.clone();

        let sources = scanner.scan_sources(Path::new("."));
        let has_sources = !sources.is_empty();

        let include_dirs = scanner.scan_include_dirs(Path::new("."));

        let include_dirs_text = include_dirs
            .iter()
            .map(|d| {
                if d == "." {
                    format!("    ${{CMAKE_CURRENT_SOURCE_DIR}}")
                } else {
                    format!(
                        "    ${{CMAKE_CURRENT_SOURCE_DIR}}/{}",
                        d.strip_prefix("./").unwrap_or(d)
                    )
                }
            })
            .collect::<Vec<String>>()
            .join("\n");

        if has_sources {
            let static_name = CMakeGenerator::pascal_case(&format!("{}_{}", namespace, name));
            let src_list = sources.join("\n    ");

            let dep_links = if cpkg.dependencies.is_empty() {
                String::new()
            } else {
                cpkg.dependencies
                    .iter()
                    .map(|d| {
                        format!(
                            "target_link_libraries({static_name} PUBLIC {dep})",
                            static_name = static_name,
                            dep = d
                        )
                    })
                    .collect::<Vec<String>>()
                    .join("\n")
            };

            format!(
                r#"add_library({static_name} STATIC
    {src_list}
)

target_include_directories({static_name}
    PUBLIC
{include_dirs_text}
)

# link dependencies if any
{dep_links}

# alias for external use
add_library({alias_name} ALIAS {static_name})
"#,
                static_name = static_name,
                alias_name = alias_name,
                src_list = src_list,
                dep_links = dep_links,
                include_dirs_text = include_dirs_text
            )
        } else {
            let interface_name = format!("__{}_{}", namespace, name);

            let dep_links = if cpkg.dependencies.is_empty() {
                String::new()
            } else {
                cpkg.dependencies
                    .iter()
                    .map(|d| {
                        format!(
                            "target_link_libraries({interface_name} INTERFACE {dep})",
                            interface_name = interface_name,
                            dep = d
                        )
                    })
                    .collect::<Vec<String>>()
                    .join("\n")
            };

            format!(
                r#"add_library({interface_name} INTERFACE)

target_include_directories({interface_name} INTERFACE
{include_dirs_text}
)

# link dependencies if any
{dep_links}

# alias for external use
add_library({alias_name} ALIAS {interface_name})
"#,
                interface_name = interface_name,
                alias_name = alias_name,
                dep_links = dep_links,
                include_dirs_text = include_dirs_text
            )
        }
    }
}
