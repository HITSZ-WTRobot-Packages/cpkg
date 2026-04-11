pub mod package;
pub mod project;

pub use package::{
    CMakeGenerator, Cpkg, DefaultFsScanner, Generator, Scanner, create as create_package,
    generate as generate_package, init as init_package,
};
pub use project::{
    ProjectInitOptions, SubmoduleProtocol, SyncOptions, SyncSummary, WtrProject,
    add as add_packages, add_and_sync as add_packages_and_sync,
    add_interactive as add_packages_interactive, init as init_project,
    init_interactive as init_project_interactive, remove as remove_packages, sync as sync_project,
};
