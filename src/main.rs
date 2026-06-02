use clap::Parser;

use pdf_rs::cli::Cli;
use pdf_rs::pipeline;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = pipeline::run(&cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
