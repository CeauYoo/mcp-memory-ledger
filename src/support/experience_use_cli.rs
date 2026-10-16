//! Explicit local experience-use reports. Never bootstraps or migrates a database.
use crate::{
    adapters::sqlite::open_current_database,
    application::experience_use,
    domain::experience_use::{
        DeriveExperienceRequest, ListExperienceUsesRequest, MAX_EXPERIENCE_USE_BYTES,
        RecordExperienceUseRequest,
    },
    support::config::AppConfig,
};

pub async fn try_run_cli(args: &[String]) -> anyhow::Result<bool> {
    if args.first().map(String::as_str) != Some("experience-use") {
        return Ok(false);
    }
    if args.len() != 4 || args[2] != "--json" {
        anyhow::bail!(
            "usage: experience-use <record|list|derive> --json '<request JSON>'; configure the intended database explicitly"
        );
    }
    let max_bytes = if args[1] == "derive" {
        crate::domain::experience::MAX_EXPERIENCE_PAYLOAD_BYTES
    } else {
        MAX_EXPERIENCE_USE_BYTES
    };
    if args[3].len() > max_bytes {
        anyhow::bail!("experience-use request exceeds {max_bytes} bytes");
    }
    // Decode before opening storage so malformed/unknown commands have no side effects.
    enum Request {
        Record(Box<RecordExperienceUseRequest>),
        Derive(Box<DeriveExperienceRequest>),
        List(ListExperienceUsesRequest),
    }
    let request = match args[1].as_str() {
        "record" => Request::Record(serde_json::from_str(&args[3])?),
        "list" => Request::List(serde_json::from_str(&args[3])?),
        "derive" => Request::Derive(serde_json::from_str(&args[3])?),
        _ => anyhow::bail!("unknown experience-use operation"),
    };
    let config = AppConfig::load().map_err(anyhow::Error::msg)?;
    let store = open_current_database(&config.database_url).await?;
    let result = match request {
        Request::Record(request) => {
            serde_json::to_value(experience_use::record_experience_use(&store, *request).await?)?
        }
        Request::Derive(request) => serde_json::to_value(
            experience_use::derive_experience_from_use(&store, *request).await?,
        )?,
        Request::List(request) => {
            serde_json::to_value(experience_use::list_experience_uses(&store, request).await?)?
        }
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[tokio::test]
    async fn ignores_other_commands_and_rejects_invalid_requests_before_storage() {
        assert!(!try_run_cli(&args(&["task"])).await.unwrap());
        for values in [
            vec!["experience-use"],
            vec!["experience-use", "record", "--json", "{}"],
            vec!["experience-use", "derive", "--json", "{}"],
            vec!["experience-use", "list", "--json", "not JSON"],
            vec!["experience-use", "unknown", "--json", "{}"],
        ] {
            assert!(try_run_cli(&args(&values)).await.is_err());
        }
        let oversized = "x".repeat(MAX_EXPERIENCE_USE_BYTES + 1);
        assert!(
            try_run_cli(&args(&["experience-use", "record", "--json", &oversized]))
                .await
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
    }
}
