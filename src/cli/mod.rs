pub mod args;
pub mod commands;
pub mod output;

use crate::errors::TgError;

use self::args::{Cli, Commands};

pub fn dispatch(cli: Cli) -> Result<(), TgError> {
    match cli.command {
        Commands::Init { force } => commands::init::run(cli.json, force),
    }
}
