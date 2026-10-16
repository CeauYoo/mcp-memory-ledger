use agent_llm_mm::{
    RunOutput, run_command,
    support::{cli::command_from_args, config::AppConfig},
};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    agent_llm_mm::support::tracing::init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if agent_llm_mm::adapters::codex_actions::try_run_cli(&args).await? {
        return Ok(());
    }
    if agent_llm_mm::support::database_backup::try_run_cli(&args).await? {
        return Ok(());
    }
    if agent_llm_mm::support::experience_use_cli::try_run_cli(&args).await? {
        return Ok(());
    }
    if agent_llm_mm::support::task_cli::try_run_cli(&args).await? {
        return Ok(());
    }
    if agent_llm_mm::adapters::codex_context::try_run_cli(&args).await? {
        return Ok(());
    }
    if let Some(command) = agent_llm_mm::interfaces::local_core::cli::parse_args(std::env::args())?
    {
        return agent_llm_mm::interfaces::local_core::run_command(command).await;
    }
    let command = command_from_args(std::env::args())?;
    tracing::debug!(?command, "CLI command selected");
    let config = AppConfig::load().map_err(anyhow::Error::msg)?;
    match run_command(command, config).await? {
        Some(RunOutput::Doctor(report)) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some(RunOutput::DatabaseLifecycle(report)) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        None => Ok(()),
    }
}
