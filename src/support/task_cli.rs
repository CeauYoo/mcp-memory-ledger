//! Explicit local task administration. Uses the current database; never bootstraps it.
use crate::{
    adapters::sqlite::open_current_database, application::task, support::config::AppConfig,
};
use serde::de::DeserializeOwned;
fn decode<T: DeserializeOwned>(value: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(value)?)
}
pub async fn try_run_cli(args: &[String]) -> anyhow::Result<bool> {
    if args.first().map(String::as_str) != Some("task") {
        return Ok(false);
    }
    if args.len() != 4 || args[2] != "--json" {
        anyhow::bail!(
            "usage: task <create|revise|get|list|checkpoint|context> --json '<request JSON>'; configure the intended database explicitly"
        );
    }
    if args[3].len() > crate::domain::task::MAX_TASK_BYTES {
        anyhow::bail!("task request exceeds 131072 bytes");
    }
    let config = AppConfig::load().map_err(anyhow::Error::msg)?;
    let store = open_current_database(&config.database_url).await?;
    let result = match args[1].as_str() {
        "create" => serde_json::to_value(task::create_task(&store, decode(&args[3])?).await?)?,
        "revise" => serde_json::to_value(task::revise_task(&store, decode(&args[3])?).await?)?,
        "get" => serde_json::to_value(task::get_task(&store, decode(&args[3])?).await?)?,
        "list" => serde_json::to_value(task::list_tasks(&store, decode(&args[3])?).await?)?,
        "checkpoint" => {
            serde_json::to_value(task::record_task_checkpoint(&store, decode(&args[3])?).await?)?
        }
        "context" => {
            serde_json::to_value(task::build_task_resume_context(&store, decode(&args[3])?).await?)?
        }
        _ => anyhow::bail!("unknown task operation"),
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(true)
}
