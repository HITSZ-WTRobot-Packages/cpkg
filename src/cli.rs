use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::Path;

use cpkg::{
    IndexSourceConfig, ProjectInitOptions, SubmoduleProtocol, SyncOptions, add_global_index_source,
    add_packages_and_sync, add_packages_interactive, clear_global_default_org_source,
    create_package, generate_package, init_global_config, init_package, init_project,
    init_project_interactive, move_global_index_source, project::write_init_integration_guidance,
    remove_global_index_source, remove_global_org_source, remove_packages,
    set_global_default_org_source, set_global_index_source, set_global_org_source,
    show_global_config, show_global_index_sources, sync_project,
};

#[derive(Parser)]
#[command(
    author,
    version,
    about = "STM32CubeMX package manager for WTR projects",
    long_about = "cpkg manages STM32CubeMX-based firmware projects and WTR driver packages.\n\n\
The project-side workflow uses `wtrproject.toml` to track direct dependencies, \
downloads a package index, resolves transitive dependencies, and synchronizes \
driver repositories into `Modules/` as Git submodules.\n\n\
Package-authoring commands stay under `cpkg package ...` and continue to manage \
individual driver-package metadata with `cpkg.toml`.",
    after_help = "Examples:\n  \
cpkg init --ioc MyBoard.ioc\n  \
cpkg init -I\n  \
cpkg add MotorDrivers::DJI bsp::CANDriver\n  \
cpkg add --offline MotorDrivers::DJI\n  \
cpkg add -I --submodule-protocol https\n  \
cpkg sync --submodule-protocol ssh\n  \
cpkg sync --offline\n  \
cpkg config init\n  \
cpkg config show\n  \
cpkg config index list\n  \
cpkg config index add --url https://mirror.example.com/cpkg_index.json\n  \
cpkg config org default set wtr-github\n  \
cpkg config org set wtr-github --default-protocol https\n  \
cpkg package init MotorDrivers::DJI --deps bsp::CANDriver"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize `wtrproject.toml` for the current STM32CubeMX project.
    Init(ProjectInitArgs),
    /// Add direct dependencies, or edit them interactively, and synchronize `Modules/`.
    Add(AddArgs),
    /// Remove direct package dependencies and refresh local project links.
    Remove(RemoveArgs),
    /// Synchronize submodules and regenerate project integration files.
    Sync(SyncArgs),
    /// Show or update global mirror configuration under `~/.cpkg/config.toml`.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Driver-package authoring commands for `cpkg.toml`.
    Package {
        #[command(subcommand)]
        command: PackageCommands,
    },
}

#[derive(Args)]
#[command(
    about = "Initialize `wtrproject.toml` in the current STM32CubeMX project",
    after_help = "Examples:\n  \
cpkg init --ioc MyBoard.ioc\n  \
cpkg init --name hero_chassis --ioc Hero.ioc\n  \
cpkg init -I\n\n\
After `cpkg sync`, include `cmake/wtr_modules.cmake` from the root `CMakeLists.txt`, \
then call `wtr_link_packages(<target>)` or `wtr_link_packages_public(<target>)`."
)]
struct ProjectInitArgs {
    /// Overwrite an existing `wtrproject.toml`.
    #[arg(short, long)]
    force: bool,
    /// Open an interactive tree picker for the initial dependencies.
    #[arg(short = 'I', long)]
    interactive: bool,
    /// Explicit project name to write into `wtrproject.toml`.
    #[arg(long)]
    name: Option<String>,
    /// Explicit `.ioc` file to bind to this project.
    #[arg(long)]
    ioc: Option<String>,
}

#[derive(Args)]
#[command(
    about = "Add direct package dependencies, or edit them interactively, and synchronize submodules",
    after_help = "Examples:\n  \
cpkg add MotorDrivers::DJI\n  \
cpkg add MotorDrivers::DJI bsp::CANDriver --submodule-protocol ssh\n  \
cpkg add --offline MotorDrivers::DJI\n  \
cpkg add -I --submodule-protocol https\n  \
cpkg add -I MotorDrivers::DJI\n\n\
If `cpkg add --offline` records a dependency that cannot be applied without fetching a new \
repository, it still updates `wtrproject.toml`; run `cpkg sync` online later to apply it."
)]
struct AddArgs {
    /// Edit direct dependencies in an interactive tree picker.
    #[arg(short = 'I', long)]
    interactive: bool,
    #[command(flatten)]
    sync: SyncOptionArgs,
    /// Direct package names to add, or preselect when using `-I`.
    #[arg(value_name = "PACKAGE", required_unless_present = "interactive")]
    packages: Vec<String>,
}

#[derive(Args)]
#[command(
    about = "Remove direct package dependencies and refresh local project links",
    after_help = "Examples:\n  \
cpkg remove MotorDrivers::DJI\n  \
cpkg remove MotorDrivers::DJI bsp::CANDriver\n\n\
This command updates `wtrproject.toml` and regenerates `cmake/wtr_modules.cmake` locally \
without synchronizing `Modules/` submodules."
)]
struct RemoveArgs {
    /// Direct package names to remove from `wtrproject.toml`.
    #[arg(value_name = "PACKAGE", required = true)]
    packages: Vec<String>,
}

#[derive(Args)]
#[command(
    about = "Synchronize `Modules/` submodules and regenerate CMake integration",
    after_help = "Examples:\n  \
cpkg sync\n  \
cpkg sync --submodule-protocol https\n  \
cpkg sync --offline\n\n\
This command generates `cmake/wtr_modules.cmake`; include it from the root `CMakeLists.txt` \
and call `wtr_link_packages(<target>)` or `wtr_link_packages_public(<target>)`."
)]
struct SyncArgs {
    #[command(flatten)]
    sync: SyncOptionArgs,
}

#[derive(Args, Clone, Copy)]
#[command(next_help_heading = "Sync Options")]
struct SyncOptionArgs {
    /// Override the protocol used when adding or updating Git submodule remotes.
    #[arg(long, value_enum)]
    submodule_protocol: Option<SubmoduleProtocolArg>,
    /// Use the project-local or cached package index and skip Git fetch/pull operations.
    #[arg(long)]
    offline: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SubmoduleProtocolArg {
    /// Use HTTPS remotes such as `https://github.com/...`.
    Https,
    /// Use SSH remotes such as `git@github.com:...`.
    Ssh,
}

impl From<SubmoduleProtocolArg> for SubmoduleProtocol {
    fn from(value: SubmoduleProtocolArg) -> Self {
        match value {
            SubmoduleProtocolArg::Https => SubmoduleProtocol::Https,
            SubmoduleProtocolArg::Ssh => SubmoduleProtocol::Ssh,
        }
    }
}

impl From<SyncOptionArgs> for SyncOptions {
    fn from(value: SyncOptionArgs) -> Self {
        SyncOptions {
            submodule_protocol: value.submodule_protocol.map(Into::into),
            offline: value.offline,
        }
    }
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Create `~/.cpkg/config.toml` from the built-in template.
    Init(ConfigInitArgs),
    /// Print the current global config file path and contents.
    Show,
    /// Create, update, remove, or reorder global index sources.
    Index {
        #[command(subcommand)]
        command: IndexConfigCommands,
    },
    /// Create, update, or remove named global org sources.
    Org {
        #[command(subcommand)]
        command: OrgConfigCommands,
    },
}

#[derive(Args)]
struct ConfigInitArgs {
    /// Overwrite an existing global config file.
    #[arg(short, long)]
    force: bool,
}

#[derive(Subcommand)]
enum IndexConfigCommands {
    /// List global index sources in the order they are tried.
    List,
    /// Add a new global index source.
    Add(AddIndexSourceArgs),
    /// Replace a global index source at a specific position.
    Set(SetIndexSourceArgs),
    /// Remove a global index source by its 1-based position.
    Remove {
        /// 1-based position of the index source to remove.
        position: usize,
    },
    /// Move a global index source from one 1-based position to another.
    Move {
        /// Current 1-based position of the index source.
        from: usize,
        /// New 1-based position of the index source.
        to: usize,
    },
}

#[derive(Args, Clone)]
struct IndexSourceArgs {
    /// Local package index file path.
    #[arg(long, conflicts_with = "url")]
    path: Option<String>,
    /// Remote package index URL.
    #[arg(long, conflicts_with = "path")]
    url: Option<String>,
    /// Cache path used when `--url` is set.
    #[arg(long, requires = "url")]
    cache_path: Option<String>,
}

impl From<IndexSourceArgs> for IndexSourceConfig {
    fn from(value: IndexSourceArgs) -> Self {
        Self {
            path: value.path,
            url: value.url,
            cache_path: value.cache_path,
        }
    }
}

#[derive(Args)]
#[command(
    about = "Add a new global index source",
    after_help = "Examples:\n  \
cpkg config index add --url https://mirror.example.com/cpkg_index.json\n  \
cpkg config index add --path /tmp/cpkg_index.json --position 1"
)]
struct AddIndexSourceArgs {
    #[command(flatten)]
    source: IndexSourceArgs,
    /// Insert position in the global fallback order; defaults to appending.
    #[arg(long)]
    position: Option<usize>,
}

#[derive(Args)]
#[command(
    about = "Replace a global index source at a specific position",
    after_help = "Examples:\n  \
cpkg config index set 1 --url https://mirror.example.com/cpkg_index.json --cache-path indexes/mirror.json\n  \
cpkg config index set 2 --path /tmp/cpkg_index.json"
)]
struct SetIndexSourceArgs {
    /// 1-based position of the index source to replace.
    position: usize,
    #[command(flatten)]
    source: IndexSourceArgs,
}

#[derive(Subcommand)]
enum OrgConfigCommands {
    /// Create or update a named global org source.
    Set(SetOrgSourceArgs),
    /// Remove a named global org source.
    Remove {
        /// Name of the global org source to remove.
        name: String,
    },
    /// Set or clear the named global org used by default.
    Default {
        #[command(subcommand)]
        command: DefaultOrgConfigCommands,
    },
}

#[derive(Subcommand)]
enum DefaultOrgConfigCommands {
    /// Use the named global org source by default.
    Set {
        /// Name of the global org source to use by default.
        name: String,
    },
    /// Clear the explicit global default org source.
    Clear,
}

#[derive(Args)]
#[command(
    about = "Create or update a named global org source",
    after_help = "Examples:\n  \
cpkg config org set wtr-github --ssh-base git@github.com:HITSZ-WTRobot-Packages --https-base https://github.com/HITSZ-WTRobot-Packages\n  \
cpkg config org set wtr-github --default-protocol https"
)]
struct SetOrgSourceArgs {
    /// Logical name of the org source.
    name: String,
    /// SSH repository base, such as `git@github.com:your-org`.
    #[arg(long)]
    ssh_base: Option<String>,
    /// HTTPS repository base, such as `https://github.com/your-org`.
    #[arg(long)]
    https_base: Option<String>,
    /// Default protocol to use when a project references this org source.
    #[arg(long, value_enum)]
    default_protocol: Option<SubmoduleProtocolArg>,
}

#[derive(Subcommand)]
enum PackageCommands {
    /// Create or migrate `cpkg.toml` and generate `CMakeLists.txt`.
    Init {
        /// Overwrite an existing `CMakeLists.txt`.
        #[arg(short, long)]
        force: bool,
        /// Package target name, such as `MotorDrivers::DJI`.
        pkgname: String,
        /// Direct package dependencies to record in `cpkg.toml`.
        #[arg(short, long)]
        deps: Vec<String>,
    },
    /// Regenerate `CMakeLists.txt` from the local `cpkg.toml`.
    Generate,
    /// Scaffold a new driver-package directory with `include/` and `src/`.
    Create {
        /// Folder name for the new package scaffold.
        package_name: String,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = Path::new(".");

    match cli.command {
        Commands::Init(args) => {
            let options = ProjectInitOptions {
                force: args.force,
                name: args.name,
                ioc: args.ioc,
            };
            let manifest = if args.interactive {
                init_project_interactive(cwd, options)?
            } else {
                Some(init_project(cwd, options)?)
            };
            if let Some(manifest) = manifest {
                write_init_integration_guidance(&manifest)?;
            }
        }
        Commands::Add(args) => {
            if args.interactive {
                add_packages_interactive(cwd, &args.packages, args.sync.into())?;
            } else {
                add_packages_and_sync(cwd, &args.packages, args.sync.into())?;
            }
        }
        Commands::Remove(args) => {
            remove_packages(cwd, &args.packages)?;
        }
        Commands::Sync(args) => {
            sync_project(cwd, args.sync.into())?;
        }
        Commands::Config { command } => match command {
            ConfigCommands::Init(args) => init_global_config(args.force)?,
            ConfigCommands::Show => show_global_config()?,
            ConfigCommands::Index { command } => match command {
                IndexConfigCommands::List => show_global_index_sources()?,
                IndexConfigCommands::Add(args) => {
                    add_global_index_source(args.source.into(), args.position)?
                }
                IndexConfigCommands::Set(args) => {
                    set_global_index_source(args.position, args.source.into())?
                }
                IndexConfigCommands::Remove { position } => remove_global_index_source(position)?,
                IndexConfigCommands::Move { from, to } => move_global_index_source(from, to)?,
            },
            ConfigCommands::Org { command } => match command {
                OrgConfigCommands::Set(args) => set_global_org_source(
                    &args.name,
                    args.ssh_base,
                    args.https_base,
                    args.default_protocol.map(Into::into),
                )?,
                OrgConfigCommands::Remove { name } => remove_global_org_source(&name)?,
                OrgConfigCommands::Default { command } => match command {
                    DefaultOrgConfigCommands::Set { name } => set_global_default_org_source(&name)?,
                    DefaultOrgConfigCommands::Clear => clear_global_default_org_source()?,
                },
            },
        },
        Commands::Package { command } => match command {
            PackageCommands::Init {
                pkgname,
                force,
                deps,
            } => init_package(cwd, &pkgname, force, &deps)?,
            PackageCommands::Generate => generate_package(cwd)?,
            PackageCommands::Create { package_name } => create_package(cwd, &package_name)?,
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, Commands, ConfigCommands, DefaultOrgConfigCommands, IndexConfigCommands,
        OrgConfigCommands,
    };
    use clap::Parser;

    #[test]
    fn add_accepts_offline_sync_option() {
        let cli = Cli::try_parse_from(["cpkg", "add", "--offline", "MotorDrivers::DJI"]).unwrap();

        match cli.command {
            Commands::Add(args) => {
                assert!(args.sync.offline);
                assert!(args.sync.submodule_protocol.is_none());
                assert_eq!(args.packages, vec!["MotorDrivers::DJI"]);
            }
            _ => panic!("expected add command"),
        }
    }

    #[test]
    fn sync_accepts_offline_sync_option() {
        let cli = Cli::try_parse_from(["cpkg", "sync", "--offline"]).unwrap();

        match cli.command {
            Commands::Sync(args) => assert!(args.sync.offline),
            _ => panic!("expected sync command"),
        }
    }

    #[test]
    fn add_accepts_submodule_protocol_override() {
        let cli = Cli::try_parse_from([
            "cpkg",
            "add",
            "--submodule-protocol",
            "https",
            "MotorDrivers::DJI",
        ])
        .unwrap();

        match cli.command {
            Commands::Add(args) => assert!(matches!(
                args.sync.submodule_protocol,
                Some(super::SubmoduleProtocolArg::Https)
            )),
            _ => panic!("expected add command"),
        }
    }

    #[test]
    fn config_org_set_accepts_named_source_updates() {
        let cli = Cli::try_parse_from([
            "cpkg",
            "config",
            "org",
            "set",
            "mirror",
            "--default-protocol",
            "https",
        ])
        .unwrap();

        match cli.command {
            Commands::Config {
                command:
                    ConfigCommands::Org {
                        command: OrgConfigCommands::Set(args),
                    },
            } => {
                assert_eq!(args.name, "mirror");
                assert!(matches!(
                    args.default_protocol,
                    Some(super::SubmoduleProtocolArg::Https)
                ));
            }
            _ => panic!("expected config org set command"),
        }
    }

    #[test]
    fn config_init_accepts_force() {
        let cli = Cli::try_parse_from(["cpkg", "config", "init", "--force"]).unwrap();

        match cli.command {
            Commands::Config {
                command: ConfigCommands::Init(args),
            } => assert!(args.force),
            _ => panic!("expected config init command"),
        }
    }

    #[test]
    fn config_index_add_accepts_position_and_remote_source() {
        let cli = Cli::try_parse_from([
            "cpkg",
            "config",
            "index",
            "add",
            "--url",
            "https://mirror.example.com/cpkg_index.json",
            "--cache-path",
            "indexes/mirror.json",
            "--position",
            "1",
        ])
        .unwrap();

        match cli.command {
            Commands::Config {
                command:
                    ConfigCommands::Index {
                        command: IndexConfigCommands::Add(args),
                    },
            } => {
                assert_eq!(args.position, Some(1));
                assert_eq!(
                    args.source.url.as_deref(),
                    Some("https://mirror.example.com/cpkg_index.json")
                );
                assert_eq!(
                    args.source.cache_path.as_deref(),
                    Some("indexes/mirror.json")
                );
            }
            _ => panic!("expected config index add command"),
        }
    }

    #[test]
    fn config_index_move_accepts_positions() {
        let cli = Cli::try_parse_from(["cpkg", "config", "index", "move", "3", "1"]).unwrap();

        match cli.command {
            Commands::Config {
                command:
                    ConfigCommands::Index {
                        command: IndexConfigCommands::Move { from, to },
                    },
            } => {
                assert_eq!(from, 3);
                assert_eq!(to, 1);
            }
            _ => panic!("expected config index move command"),
        }
    }

    #[test]
    fn config_org_default_set_accepts_name() {
        let cli =
            Cli::try_parse_from(["cpkg", "config", "org", "default", "set", "mirror"]).unwrap();

        match cli.command {
            Commands::Config {
                command:
                    ConfigCommands::Org {
                        command:
                            OrgConfigCommands::Default {
                                command: DefaultOrgConfigCommands::Set { name },
                            },
                    },
            } => assert_eq!(name, "mirror"),
            _ => panic!("expected config org default set command"),
        }
    }
}
