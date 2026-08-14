use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cli::args::WorkflowAction;
use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::store::Store;
use task_golem::store::config::Config;
use task_golem::store::root;
use task_golem::workflow::instantiate::{WorkflowInstance, instantiate_workflow};
use task_golem::workflow::runner::{WorkflowRunOutcome, run_campaign_state_shell};
use task_golem::workflow::template::load_workflow_definition;

pub fn run(json_mode: bool, action: WorkflowAction) -> Result<(), TgError> {
    match action {
        WorkflowAction::Instantiate {
            template,
            instance,
            inputs,
        } => instantiate(json_mode, template, instance, inputs),
        WorkflowAction::Run { campaign_id } => run_campaign(json_mode, campaign_id),
    }
}

fn run_campaign(json_mode: bool, campaign_id: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);
    let results_dir = store.workflow_results_dir();
    std::fs::create_dir_all(&results_dir).map_err(TgError::IoError)?;
    let lock_path = campaign_lock_path(&results_dir, &campaign_id);
    let outcome = task_golem::store::lock::with_lock_nonblocking(&lock_path, || {
        run_campaign_state_shell(&store, &campaign_id)
    })?;

    if json_mode {
        output::print_json(&outcome);
    } else {
        output::print_human(&run_human_output(&outcome));
    }
    Ok(())
}

pub fn campaign_lock_path(results_dir: &Path, campaign_id: &str) -> PathBuf {
    let digest = Sha256::digest(campaign_id.as_bytes());
    results_dir.join(format!("campaign-{digest:x}.lock"))
}

fn run_human_output(outcome: &WorkflowRunOutcome) -> String {
    let mut lines = vec![
        format!("Campaign: {}", outcome.campaign_id),
        format!("Outcome: {}", outcome.outcome),
    ];
    if let Some(task_id) = &outcome.task_id {
        lines.push(format!("Task: {task_id}"));
    }
    lines.join("\n")
}

fn instantiate(
    json_mode: bool,
    template: PathBuf,
    instance: String,
    inputs: Vec<String>,
) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let config = Config::load(&project_dir)?;
    let definition = load_workflow_definition(&project_dir, &template, &inputs)?;
    let result = instantiate_workflow(&Store::new(project_dir), &config, &definition, &instance)?;

    if json_mode {
        output::print_json(&result);
    } else {
        output::print_human(&human_output(&result));
    }
    Ok(())
}

fn human_output(instance: &WorkflowInstance) -> String {
    let mut lines = vec![
        format!("Campaign: {}", instance.campaign_id),
        format!("Instance: {}", instance.instance),
        format!("Digest: {}", instance.instance_digest),
        "Nodes:".to_string(),
    ];
    lines.extend(
        instance
            .nodes
            .iter()
            .map(|(node, item_id)| format!("  {node}: {item_id}")),
    );
    lines.join("\n")
}
