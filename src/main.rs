use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::Path;
use tracing_subscriber;

use cpkg::{
    ProjectInitOptions, add_packages, create_package, generate_package, init_package, init_project,
    remove_packages, sync_project,
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
    Sync,
    Package {
        #[command(subcommand)]
        command: PackageCommands,
    },
}

#[derive(Args)]
struct ProjectInitArgs {
    #[arg(short, long)]
    force: bool,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    ioc: Option<String>,
}

#[derive(Args)]
struct PackageListArgs {
    packages: Vec<String>,
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
            init_project(
                cwd,
                ProjectInitOptions {
                    force: args.force,
                    name: args.name,
                    ioc: args.ioc,
                },
            )?;
        }
        Commands::Add(args) => {
            add_packages(cwd, &args.packages)?;
        }
        Commands::Remove(args) => {
            remove_packages(cwd, &args.packages)?;
        }
        Commands::Sync => {
            sync_project(cwd)?;
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
