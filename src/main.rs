mod cli;

use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    cli::run()
}
