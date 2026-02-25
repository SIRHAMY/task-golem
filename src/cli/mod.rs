pub mod args;
pub mod commands;
pub mod output;

use crate::errors::TgError;

use self::args::{Cli, Commands};

pub fn dispatch(cli: Cli) -> Result<(), TgError> {
    match cli.command {
        Commands::Init { force } => commands::init::run(cli.json, force),
        Commands::Add {
            title,
            description,
            priority,
            deps,
            tags,
            sets,
        } => commands::add::run(cli.json, title, description, priority, deps, tags, sets),
        Commands::List { status, tag } => commands::list::run(cli.json, status, tag),
        Commands::Show { id } => commands::show::run(cli.json, id),
        Commands::Edit {
            id,
            title,
            priority,
            description,
            add_deps,
            rm_deps,
            add_tags,
            rm_tags,
            sets,
        } => commands::edit::run(
            cli.json, id, title, priority, description, add_deps, rm_deps, add_tags, rm_tags,
            sets,
        ),
        Commands::Rm {
            id,
            force,
            clear_deps,
        } => commands::rm::run(cli.json, id, force, clear_deps),
    }
}
