use std::fs;
use std::path::Path;

use super::manifest::Cpkg;
use super::scanner::Scanner;

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

            let compile_options = if cpkg.compile.options.is_empty() {
                String::new()
            } else {
                let opts = cpkg
                    .compile
                    .options
                    .iter()
                    .map(|o| format!("    {}", o))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "target_compile_options({static_name} PRIVATE\n{opts}\n)\n\n",
                    static_name = static_name,
                    opts = opts,
                )
            };

            let compile_defines = if cpkg.compile.defines.is_empty() {
                String::new()
            } else {
                let defs = cpkg
                    .compile
                    .defines
                    .iter()
                    .map(|d| format!("    {}", d))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "target_compile_definitions({static_name} PUBLIC\n{defs}\n)\n\n",
                    static_name = static_name,
                    defs = defs,
                )
            };

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

{compile_options}{compile_defines}# link dependencies if any
{dep_links}

# alias for external use
add_library({alias_name} ALIAS {static_name})
"#,
                static_name = static_name,
                alias_name = alias_name,
                src_list = src_list,
                dep_links = dep_links,
                include_dirs_text = include_dirs_text,
                compile_options = compile_options,
                compile_defines = compile_defines,
            )
        } else {
            let interface_name = format!("__{}_{}", namespace, name);

            let compile_options = if cpkg.compile.options.is_empty() {
                String::new()
            } else {
                let opts = cpkg
                    .compile
                    .options
                    .iter()
                    .map(|o| format!("    {}", o))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "target_compile_options({interface_name} INTERFACE\n{opts}\n)\n\n",
                    interface_name = interface_name,
                    opts = opts,
                )
            };

            let compile_defines = if cpkg.compile.defines.is_empty() {
                String::new()
            } else {
                let defs = cpkg
                    .compile
                    .defines
                    .iter()
                    .map(|d| format!("    {}", d))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "target_compile_definitions({interface_name} INTERFACE\n{defs}\n)\n\n",
                    interface_name = interface_name,
                    defs = defs,
                )
            };

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

{compile_options}{compile_defines}# link dependencies if any
{dep_links}

# alias for external use
add_library({alias_name} ALIAS {interface_name})
"#,
                interface_name = interface_name,
                alias_name = alias_name,
                dep_links = dep_links,
                include_dirs_text = include_dirs_text,
                compile_options = compile_options,
                compile_defines = compile_defines,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::CompileConfig;
    use std::path::Path;

    struct MockScanner {
        sources: Vec<String>,
        include_dirs: Vec<String>,
    }

    impl Scanner for MockScanner {
        fn scan_sources(&self, _dir: &Path) -> Vec<String> {
            self.sources.clone()
        }
        fn scan_include_dirs(&self, _dir: &Path) -> Vec<String> {
            self.include_dirs.clone()
        }
    }

    fn base_cpkg() -> Cpkg {
        Cpkg {
            format_version: 1,
            name: "test".to_string(),
            pkgname: "Test::Lib".to_string(),
            version: "0.1.0".to_string(),
            dependencies: vec![],
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        }
    }

    #[test]
    fn static_lib_with_compile_options_uses_private_scope() {
        let mut cpkg = base_cpkg();
        cpkg.compile = CompileConfig {
            options: vec!["-Ofast".to_string()],
            defines: vec![],
        };

        let scanner = MockScanner {
            sources: vec!["\"src/main.c\"".to_string()],
            include_dirs: vec![".".to_string()],
        };

        let cmake = CMakeGenerator::default();
        let output = cmake.generate_string(&cpkg, &scanner);

        assert!(output.contains("target_compile_options(TestLib PRIVATE"));
        assert!(output.contains("-Ofast"));
        assert!(!output.contains("target_compile_definitions"));
    }

    #[test]
    fn static_lib_with_defines_uses_public_scope() {
        let mut cpkg = base_cpkg();
        cpkg.compile = CompileConfig {
            options: vec![],
            defines: vec!["ARM_MATH_CM4".to_string()],
        };

        let scanner = MockScanner {
            sources: vec!["\"src/main.c\"".to_string()],
            include_dirs: vec![".".to_string()],
        };

        let cmake = CMakeGenerator::default();
        let output = cmake.generate_string(&cpkg, &scanner);

        assert!(output.contains("target_compile_definitions(TestLib PUBLIC"));
        assert!(output.contains("ARM_MATH_CM4"));
        assert!(!output.contains("target_compile_options"));
    }

    #[test]
    fn interface_lib_with_compile_config_uses_interface_scope() {
        let mut cpkg = base_cpkg();
        cpkg.compile = CompileConfig {
            options: vec!["-Wall".to_string()],
            defines: vec!["USE_DSP".to_string()],
        };

        let scanner = MockScanner {
            sources: vec![],
            include_dirs: vec![".".to_string()],
        };

        let cmake = CMakeGenerator::default();
        let output = cmake.generate_string(&cpkg, &scanner);

        assert!(output.contains("target_compile_options(__Test_Lib INTERFACE"));
        assert!(output.contains("-Wall"));
        assert!(output.contains("target_compile_definitions(__Test_Lib INTERFACE"));
        assert!(output.contains("USE_DSP"));
    }

    #[test]
    fn empty_compile_config_produces_no_extra_cmake_commands() {
        let cpkg = base_cpkg();

        let scanner = MockScanner {
            sources: vec!["\"src/main.c\"".to_string()],
            include_dirs: vec![".".to_string()],
        };

        let cmake = CMakeGenerator::default();
        let output = cmake.generate_string(&cpkg, &scanner);

        assert!(!output.contains("target_compile_options"));
        assert!(!output.contains("target_compile_definitions"));
    }

    #[test]
    fn generator_output_is_stable_for_legacy_cpkg() {
        let cpkg = Cpkg {
            format_version: 1,
            name: "DJI".to_string(),
            pkgname: "MotorDrivers::DJI".to_string(),
            version: "0.1.0".to_string(),
            dependencies: vec!["bsp::CANDriver".to_string()],
            compile: CompileConfig::default(),
            ignore: Vec::new(),
        };

        let scanner = MockScanner {
            sources: vec!["\"src/dji.c\"".to_string()],
            include_dirs: vec![".".to_string(), "include".to_string()],
        };

        let cmake = CMakeGenerator::default();
        let output = cmake.generate_string(&cpkg, &scanner);

        assert!(output.contains("add_library(MotorDriversDJI STATIC"));
        assert!(output.contains("target_include_directories(MotorDriversDJI"));
        assert!(
            output.contains("target_link_libraries(MotorDriversDJI PUBLIC bsp::CANDriver)")
        );
        assert!(output.contains("add_library(MotorDrivers::DJI ALIAS MotorDriversDJI)"));
    }
}
