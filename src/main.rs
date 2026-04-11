use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io;
use std::path::Path;
use tracing_subscriber;

use cpkg::{
    ProjectInitOptions, SubmoduleProtocol, SyncOptions, add_packages_and_sync,
    add_packages_interactive, create_package, generate_package, init_package, init_project,
    init_project_interactive, remove_packages, sync_project,
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
cpkg add -I --submodule-protocol https\n  \
cpkg sync --submodule-protocol ssh\n  \
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
    /// Add direct package dependencies and synchronize `Modules/`.
    Add(AddArgs),
    /// Remove direct package dependencies from `wtrproject.toml`.
    Remove(RemoveArgs),
    /// Synchronize submodules and regenerate project integration files.
    Sync(SyncArgs),
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
cpkg init -I"
)]
struct ProjectInitArgs {
    /// Overwrite an existing `wtrproject.toml`.
    #[arg(short, long)]
    force: bool,
    /// Interactively browse repositories and choose initial dependencies.
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
    about = "Add direct package dependencies and immediately synchronize submodules",
    after_help = "Examples:\n  \
cpkg add MotorDrivers::DJI\n  \
cpkg add MotorDrivers::DJI bsp::CANDriver --submodule-protocol ssh\n  \
cpkg add -I --submodule-protocol https"
)]
struct AddArgs {
    /// Interactively browse repositories and choose packages to add.
    #[arg(short = 'I', long)]
    interactive: bool,
    #[command(flatten)]
    sync: SyncOptionArgs,
    /// Direct package names to add, such as `MotorDrivers::DJI`.
    #[arg(value_name = "PACKAGE", required_unless_present = "interactive")]
    packages: Vec<String>,
}

#[derive(Args)]
#[command(
    about = "Remove direct package dependencies from `wtrproject.toml`",
    after_help = "Examples:\n  \
cpkg remove MotorDrivers::DJI\n  \
cpkg remove MotorDrivers::DJI bsp::CANDriver"
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
cpkg sync --submodule-protocol https"
)]
struct SyncArgs {
    #[command(flatten)]
    sync: SyncOptionArgs,
}

#[derive(Args, Clone, Copy)]
#[command(next_help_heading = "Sync Options")]
struct SyncOptionArgs {
    /// Protocol used when adding or updating Git submodule remotes.
    #[arg(long, value_enum, default_value_t = SubmoduleProtocolArg::Ssh)]
    submodule_protocol: SubmoduleProtocolArg,
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
            submodule_protocol: value.submodule_protocol.into(),
        }
    }
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

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cwd = Path::new(".");

    match cli.command {
        Commands::Init(args) => {
            let options = ProjectInitOptions {
                force: args.force,
                name: args.name,
                ioc: args.ioc,
            };
            if args.interactive {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                let mut output = io::stdout();
                init_project_interactive(cwd, options, &mut input, &mut output)?;
            } else {
                init_project(cwd, options)?;
            }
        }
        Commands::Add(args) => {
            if args.interactive {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                let mut output = io::stdout();
                add_packages_interactive(
                    cwd,
                    &args.packages,
                    args.sync.into(),
                    &mut input,
                    &mut output,
                )?;
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
