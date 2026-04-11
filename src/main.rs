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
    about = "STM32CubeMX package manager for WTR projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init(ProjectInitArgs),
    Add(PackageListArgs),
    Remove(PackageListArgs),
    Sync(SyncArgs),
    Package {
        #[command(subcommand)]
        command: PackageCommands,
    },
}

#[derive(Args)]
struct ProjectInitArgs {
    #[arg(short, long)]
    force: bool,
    #[arg(short = 'I', long)]
    interactive: bool,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    ioc: Option<String>,
}

#[derive(Args)]
struct PackageListArgs {
    #[arg(short = 'I', long)]
    interactive: bool,
    #[command(flatten)]
    sync: SyncOptionArgs,
    packages: Vec<String>,
}

#[derive(Args)]
struct SyncArgs {
    #[command(flatten)]
    sync: SyncOptionArgs,
}

#[derive(Args, Clone, Copy)]
struct SyncOptionArgs {
    #[arg(long, value_enum, default_value_t = SubmoduleProtocolArg::Ssh)]
    submodule_protocol: SubmoduleProtocolArg,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SubmoduleProtocolArg {
    Https,
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
    Init {
        #[arg(short, long)]
        force: bool,
        pkgname: String,
        #[arg(short, long)]
        deps: Vec<String>,
    },
    Generate,
    Create {
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
