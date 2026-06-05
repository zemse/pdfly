use clap::Parser;

use pdf_rs::cli::{Cli, Command};
use pdf_rs::pipeline;

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
