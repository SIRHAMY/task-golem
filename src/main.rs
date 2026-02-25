mod cli;
mod errors;
mod model;
mod store;

use std::process;

use clap::Parser;

fn main() {
    let args = cli::args::Cli::parse();
    let json_mode = args.json;

    if let Err(e) = cli::dispatch(args) {
        cli::output::print_error(json_mode, &e);
        process::exit(e.exit_code());
    }
}
