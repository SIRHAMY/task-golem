use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(json_mode: bool, verbose: bool) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    if verbose {
        eprintln!("[verbose] Project root: {}", project_dir.display());
    }
    let store = Store::new(project_dir);

    let evaluation = store.dependency_evaluation()?;
    if verbose {
        eprintln!(
            "[verbose] Evaluated dependency readiness for {} active items",
            evaluation.items.len()
        );
    }

    for issue in &evaluation.integrity_issues {
        eprintln!("{}", issue.warning());
    }

    let next_item = evaluation.ready_items.into_iter().next();

    if json_mode {
        output::print_json(&next_item);
    } else if let Some(item) = &next_item {
        output::print_human(&format!(
            "{} [{}] (p:{}) {}",
            item.id, item.status, item.priority, item.title
        ));
    } else {
        output::print_human("No items ready");
    }

    Ok(())
}
