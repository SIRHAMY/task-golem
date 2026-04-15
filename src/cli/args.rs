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

        /// Parent item ID (full, bare hex, or prefix)
        #[arg(long)]
        parent: Option<String>,
    },

    /// List items
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Filter by parent ID (direct children only)
        #[arg(long)]
        parent: Option<String>,

        /// Sugar for `--status blocked` (conflicts with --status)
        #[arg(long)]
        blocked: bool,
    },

    /// Show a single item
    Show {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Include the chronological event log at the end of the detail view
        #[arg(long)]
        events: bool,
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

        /// Set a new parent ID (mutually exclusive with --parent-clear)
        #[arg(long, conflicts_with = "parent_clear")]
        parent: Option<String>,

        /// Clear the parent field (mutually exclusive with --parent)
        #[arg(long = "parent-clear")]
        parent_clear: bool,
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

    /// Show the next item to work on (highest-priority ready item)
    Next,

    /// Manage dependencies
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },

    /// Check project integrity and diagnose issues
    Doctor {
        /// Attempt to fix detected issues
        #[arg(long)]
        fix: bool,
    },

    /// Archive maintenance: recover unarchived done items, prune old entries
    Archive {
        /// Prune archive entries older than this date (ISO 8601, e.g., 2026-01-01)
        #[arg(long)]
        before: Option<String>,
    },

    /// Export all items (active + archive) in JSON or YAML format
    Dump {
        /// Output in YAML format (default: JSON)
        #[arg(long)]
        yaml: bool,
    },

    /// Run a SELECT-only SQL query against the cache
    Query {
        /// SQL to execute (required unless --schema is set)
        sql: Option<String>,

        /// Print the cache schema as Markdown instead of running a query
        #[arg(long)]
        schema: bool,

        /// Emit results as a JSON envelope (columns + rows)
        #[arg(long)]
        json: bool,

        /// Query timeout in seconds (0 = trip immediately; no upper cap)
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },

    /// Append a free-text note event to a task
    Note {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Note text (must be non-empty)
        text: String,
    },

    /// Show the chronological event log for a task
    Events {
        /// Item ID (full, bare hex, or prefix)
        id: String,

        /// Emit results as NDJSON (one event per line)
        #[arg(long)]
        json: bool,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, elvish, powershell)
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum DepAction {
    /// Add a dependency to an item
    Add {
        /// Item ID to add the dependency to
        id: String,

        /// ID of the item to depend on
        depends_on: String,
    },

    /// Remove a dependency from an item
    Rm {
        /// Item ID to remove the dependency from
        id: String,

        /// ID of the dependency to remove
        dep_id: String,
    },
}
