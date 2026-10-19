mod cli;

use anyhow::Result;

fn main() -> Result<()> {
    if cpkg::update_check::run_if_worker() {
        return Ok(());
    }
    cli::run()
}
