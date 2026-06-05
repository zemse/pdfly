use clap::Parser;

use pdfly::cli::{Cli, Command};
use pdfly::pipeline;

fn main() {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Read(args) => pipeline::run_read(args),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
