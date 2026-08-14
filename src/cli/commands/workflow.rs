use crate::cli::args::WorkflowAction;
use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::store::Store;
use task_golem::store::config::Config;
use task_golem::store::root;
use task_golem::workflow::instantiate::{WorkflowInstance, instantiate_workflow};
use task_golem::workflow::template::load_workflow_definition;

pub fn run(json_mode: bool, action: WorkflowAction) -> Result<(), TgError> {
    match action {
        WorkflowAction::Instantiate {
            template,
            instance,
            inputs,
        } => instantiate(json_mode, template, instance, inputs),
    }
}

fn instantiate(
    json_mode: bool,
    template: std::path::PathBuf,
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
