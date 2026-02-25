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

    /// Add a new item
    Add {
        /// Item title (single line)
        title: String,

        /// Item description
        #[arg(long)]
        description: Option<String>,

        /// Priority (higher = more important)
        #[arg(long, default_value_t = 0)]
        priority: i64,

        /// Add a dependency (repeatable)
        #[arg(long = "dep", num_args = 1)]
        deps: Vec<String>,

        /// Add a tag (repeatable)
        #[arg(long = "tag", num_args = 1)]
        tags: Vec<String>,

        /// Set an extension field (repeatable, KEY=VALUE format)
        #[arg(long = "set", num_args = 1)]
        sets: Vec<String>,
    },

    /// List items
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },

    /// Show a single item
    Show {
        /// Item ID (full, bare hex, or prefix)
        id: String,
    },

    /// Edit an existing item
    Edit {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// New title
        #[arg(long)]
        title: Option<String>,

        /// New priority
        #[arg(long)]
        priority: Option<i64>,

        /// New description
        #[arg(long)]
        description: Option<String>,

        /// Add a dependency (repeatable)
        #[arg(long = "add-dep", num_args = 1)]
        add_deps: Vec<String>,

        /// Remove a dependency (repeatable)
        #[arg(long = "rm-dep", num_args = 1)]
        rm_deps: Vec<String>,

        /// Add a tag (repeatable)
        #[arg(long = "add-tag", num_args = 1)]
        add_tags: Vec<String>,

        /// Remove a tag (repeatable)
        #[arg(long = "rm-tag", num_args = 1)]
        rm_tags: Vec<String>,

        /// Set an extension field (repeatable, KEY=VALUE format)
        #[arg(long = "set", num_args = 1)]
        sets: Vec<String>,
    },

    /// Remove an item
    Rm {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Force remove even if other items depend on this one
        #[arg(long)]
        force: bool,

        /// Also remove this item's ID from all dependents' dep lists
        #[arg(long = "clear-deps")]
        clear_deps: bool,
    },

    /// Start working on an item (todo → doing)
    Do {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Claim this item for an agent/user
        #[arg(long)]
        claim: Option<String>,
    },

    /// Mark an item as done (todo/doing → done, archives item)
    Done {
        /// Item ID (full, bare hex, or prefix)
        id: String,
    },

    /// Return an item to todo (doing → todo)
    Todo {
        /// Item ID (full, bare hex, or prefix)
        id: String,
    },

    /// Block an item
    Block {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Reason for blocking
        #[arg(long)]
        reason: Option<String>,
    },

    /// Unblock an item (restores previous status)
    Unblock {
        /// Item ID (full, bare hex, or prefix)
        id: String,
    },

    /// Show items ready to work on
    Ready {
        /// Include stale doing items older than duration (e.g., 4h, 30m)
        #[arg(long = "include-stale")]
        include_stale: Option<String>,

        /// Limit number of results
        #[arg(long)]
        limit: Option<usize>,
    },
}
