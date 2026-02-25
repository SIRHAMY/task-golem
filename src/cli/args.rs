use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tg", about = "Agent-native work tracker")]
pub struct Cli {
    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    /// Enable verbose diagnostics on stderr
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize a new task-golem project
    Init {
        /// Force reinitialize (overwrites existing data)
        #[arg(long)]
        force: bool,
    },
}
