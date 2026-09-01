use std::io;

use task_golem::errors::TgError;
use task_golem::model::graph::{GraphApplyError, GraphApplyRequest};
use task_golem::store::Store;
use task_golem::store::root;

use crate::cli::output;

pub fn run(json_mode: bool) -> Result<(), TgError> {
    let request = serde_json::from_reader::<_, GraphApplyRequest>(io::stdin().lock())
        .map_err(|error| GraphApplyError::invalid_json(error.to_string()))?;
    let project_dir = root::find_project_root_from_cwd()?;
    let result = Store::new(project_dir).apply_graph(request)?;

    if json_mode {
        output::print_json(&result);
    } else {
        output::print_human(&format!("Created graph with {} items", result.count));
        for (reference, item_id) in result.mapping {
            output::print_human(&format!("{reference}: {item_id}"));
        }
    }

    Ok(())
}
