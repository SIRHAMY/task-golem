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
        Commands::List { status, tag } => commands::list::run(cli.json, cli.verbose, status, tag),
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
        Commands::Do { id, claim } => commands::transition::run_do(cli.json, id, claim),
        Commands::Done { id } => commands::transition::run_done(cli.json, id),
        Commands::Todo { id } => commands::transition::run_todo(cli.json, id),
        Commands::Block { id, reason } => commands::transition::run_block(cli.json, id, reason),
        Commands::Unblock { id } => commands::transition::run_unblock(cli.json, id),
        Commands::Ready {
            include_stale,
            limit,
        } => commands::ready::run(cli.json, cli.verbose, include_stale, limit),
        Commands::Next => commands::next::run(cli.json, cli.verbose),
        Commands::Dep { action } => commands::dep::run(cli.json, action),
        Commands::Doctor { fix } => commands::doctor::run(cli.json, fix),
        Commands::Archive { before } => commands::archive::run(cli.json, before),
        Commands::Dump { yaml } => commands::dump::run(yaml),
        Commands::Completions { shell } => {
            let mut cmd = <args::Cli as clap::CommandFactory>::command();
            clap_complete::generate(shell, &mut cmd, "tg", &mut std::io::stdout());
            Ok(())
        }
    }
}
