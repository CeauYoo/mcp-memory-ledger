//! Rebuildable normalized recording-time columns. Original RFC3339 text is the
//! authoritative fact; NULL normalized keys never invent a historical time.
use crate::error::AppError;
use sqlx::SqliteConnection;

/// SQLite conversion strips the fractional part before seconds conversion so
/// .999999999 cannot round into the next second. Fractions are copied as digits.
pub(super) fn expressions(column: &str) -> (String, String, String) {
    let zone_len = format!("CASE WHEN upper(substr({column},-1))='Z' THEN 1 ELSE 6 END");
    let offset = format!(
        "CASE WHEN upper(substr({column},-1))='Z' THEN 0 ELSE (CASE substr({column},-6,1) WHEN '-' THEN -1 ELSE 1 END) * (CAST(substr({column},-5,2) AS INTEGER)*3600+CAST(substr({column},-2,2) AS INTEGER)*60) END"
    );
    let fraction = format!(
        "CASE WHEN substr({column},20,1)='.' THEN substr({column},21,length({column})-20-({zone_len})) ELSE '' END"
    );
    // Parse local whole seconds independently of SQLite's narrower zone parser.
    // Chrono represents a leap second as second 59 plus >=1e9 nanoseconds.
    let whole = format!(
        "upper(substr({column},1,17))||CASE WHEN substr({column},18,2)='60' THEN '59' ELSE substr({column},18,2) END||'Z'"
    );
    let seconds = format!("CAST(strftime('%s',({whole})) AS INTEGER)-({offset})");

    let digits = format!(
        "substr({column},1,4)||substr({column},6,2)||substr({column},9,2)||substr({column},12,2)||substr({column},15,2)||substr({column},18,2)"
    );
    let valid = format!(
        "{column} IS NOT NULL AND typeof({column})='text' AND instr({column},char(0))=0 AND length({column}) BETWEEN 20 AND 35 AND substr({column},5,1)='-' AND substr({column},8,1)='-' AND upper(substr({column},11,1))='T' AND substr({column},14,1)=':' AND substr({column},17,1)=':' AND ({digits}) NOT GLOB '*[^0-9]*' AND substr({column},12,2) BETWEEN '00' AND '23' AND substr({column},15,2) BETWEEN '00' AND '59' AND substr({column},18,2) BETWEEN '00' AND '60' AND strftime('%Y-%m-%d',substr({column},1,10),'+0 days')=substr({column},1,10) AND ((length({column})=19+({zone_len}) AND substr({column},20,1)<>'.') OR (substr({column},20,1)='.' AND length({fraction}) BETWEEN 1 AND 9 AND ({fraction}) NOT GLOB '*[^0-9]*')) AND (upper(substr({column},-1))='Z' OR (substr({column},-6,1) IN ('+','-') AND substr({column},-3,1)=':' AND substr({column},-5,2) NOT GLOB '*[^0-9]*' AND substr({column},-2,2) NOT GLOB '*[^0-9]*' AND substr({column},-5,2) BETWEEN '00' AND '23' AND substr({column},-2,2) BETWEEN '00' AND '59')) AND ({seconds}) IS NOT NULL"
    );
    let fraction_digits = format!("substr(({fraction})||'000000000',1,9)");
    let nanos = format!(
        "CAST({fraction_digits} AS INTEGER)+CASE WHEN substr({column},18,2)='60' THEN 1000000000 ELSE 0 END"
    );
    (
        format!("CASE WHEN {valid} THEN ({seconds}) END"),
        format!("CASE WHEN {valid} THEN ({nanos}) END"),
        format!(
            "CASE WHEN {valid} THEN printf('%020d:%010d',({seconds})+10000000000000,({nanos})) END"
        ),
    )
}

pub(super) fn ddl() -> Vec<(String, String)> {
    let mut statements = Vec::new();
    for (table, id) in [
        ("events", "event_id"),
        ("claims", "claim_id"),
        ("reflections", "reflection_id"),
    ] {
        let (seconds, nanos, key) = expressions("new.recorded_at");
        for (suffix, event) in [("ai", "INSERT"), ("au", "UPDATE OF recorded_at")] {
            let name = format!("recorded_time_{table}_{suffix}");
            statements.push((name.clone(),format!("CREATE TRIGGER {name} AFTER {event} ON {table} BEGIN UPDATE {table} SET recorded_at_seconds={seconds}, recorded_at_nanos={nanos}, recorded_at_sort_key={key} WHERE {id}=new.{id}; END")));
        }
    }
    for (name, sql) in [
        (
            "idx_events_scope_recorded",
            "CREATE INDEX idx_events_scope_recorded ON events(owner, namespace, recorded_at_sort_key DESC, event_id)",
        ),
        (
            "idx_claims_scope_recorded",
            "CREATE INDEX idx_claims_scope_recorded ON claims(owner, namespace, recorded_at_sort_key DESC, claim_id)",
        ),
        (
            "idx_reflections_recorded",
            "CREATE INDEX idx_reflections_recorded ON reflections(recorded_at_sort_key DESC, reflection_id)",
        ),
    ] {
        statements.push((name.into(), sql.into()));
    }
    statements
}

pub(super) async fn install(connection: &mut SqliteConnection) -> Result<(), AppError> {
    for (_, sql) in ddl() {
        sqlx::query(&sql)
            .execute(&mut *connection)
            .await
            .map_err(|e| AppError::Message(e.to_string()))?;
    }
    for table in ["events", "claims", "reflections"] {
        let (seconds, nanos, key) = expressions("recorded_at");
        sqlx::query(&format!("UPDATE {table} SET recorded_at_seconds={seconds}, recorded_at_nanos={nanos}, recorded_at_sort_key={key}"))
            .execute(&mut *connection).await.map_err(|e|AppError::Message(e.to_string()))?;
    }
    Ok(())
}
