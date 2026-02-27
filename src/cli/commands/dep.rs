use crate::cli::args::DepAction;
use task_golem::errors::TgError;

/// Sugar commands for dependency management. Delegate to edit logic.
pub fn run(json_mode: bool, action: DepAction) -> Result<(), TgError> {
    match action {
        DepAction::Add { id, depends_on } => {
            // Delegate to edit with --add-dep
            super::edit::run(
                json_mode,
                id,
                None,           // title
                None,           // priority
                None,           // description
                vec![depends_on], // add_deps
                vec![],         // rm_deps
                vec![],         // add_tags
                vec![],         // rm_tags
                vec![],         // sets
            )
        }
        DepAction::Rm { id, dep_id } => {
            // Delegate to edit with --rm-dep
            super::edit::run(
                json_mode,
                id,
                None,         // title
                None,         // priority
                None,         // description
                vec![],       // add_deps
                vec![dep_id], // rm_deps
                vec![],       // add_tags
                vec![],       // rm_tags
                vec![],       // sets
            )
        }
    }
}
