//! Postgres through sqlx. Queries build JSON in Postgres (`row_to_json` /
//! `json_agg`) wherever a handler just returns rows, so there is one
//! round trip and no per-column decoding in Rust.

use crate::config::Config;
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};
use std::time::Duration;

pub async fn connect(cfg: &Config) -> anyhow::Result<PgPool> {
    let opts = PgConnectOptions::new()
        .host(&cfg.db_host)
        .port(cfg.db_port)
        .username(&cfg.db_user)
        .password(&cfg.db_password)
        .database(&cfg.db_name)
        .application_name("lumisound-bridge")
        // Slow queries land in the log (and so in front of whoever reads it).
        .log_slow_statements(log::LevelFilter::Warn, Duration::from_millis(300))
        .log_statements(log::LevelFilter::Trace);
    let pool = PgPoolOptions::new()
        .max_connections(cfg.db_pool_max)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .connect_lazy_with(opts);
    Ok(pool)
}

/// Versioned migrations, applied once each in file order (`migrations/`).
/// New schema goes here, not into schema.sql.
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0002_stream_tickets", include_str!("../migrations/0002_stream_tickets.sql")),
    ("0003_download_retry", include_str!("../migrations/0003_download_retry.sql")),
    ("0004_track_profiles", include_str!("../migrations/0004_track_profiles.sql")),
    ("0006_metadata_since", include_str!("../migrations/0006_metadata_since.sql")),
    ("0007_favorite_keys", include_str!("../migrations/0007_favorite_keys.sql")),
];

/// Brings the database up to date in one transaction under an advisory lock:
/// schema.sql (the idempotent baseline) runs only when its checksum differs
/// from the one recorded last time, then every migration not yet applied.
/// An unchanged start is a single SELECT instead of ~2,000 lines of DDL
/// (and ~150 "already exists, skipping" notices in the log).
pub async fn migrate(pool: &PgPool, schema_sql: &str) -> anyhow::Result<()> {
    use sha2::Digest;
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(
        "SET LOCAL client_min_messages = warning; \
         SELECT pg_advisory_xact_lock(7742001); \
         CREATE TABLE IF NOT EXISTS lumi_schema_migrations ( \
             name TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at TIMESTAMP NOT NULL DEFAULT (NOW() AT TIME ZONE 'UTC'))",
    )
    .execute(&mut *tx)
    .await?;
    let applied: Vec<(String, String)> = sqlx::query_as("SELECT name, checksum FROM lumi_schema_migrations").fetch_all(&mut *tx).await?;
    let known = |name: &str| applied.iter().find(|(n, _)| n == name).map(|(_, c)| c.clone());
    let sum = |sql: &str| hex::encode(sha2::Sha256::digest(sql.as_bytes()));
    let mut ran: Vec<&str> = Vec::new();
    let base_sum = sum(schema_sql);
    if known("0001_schema.sql").as_deref() != Some(base_sum.as_str()) {
        sqlx::raw_sql(schema_sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO lumi_schema_migrations (name, checksum) VALUES ('0001_schema.sql', $1) \
                     ON CONFLICT (name) DO UPDATE SET checksum = EXCLUDED.checksum, applied_at = NOW() AT TIME ZONE 'UTC'")
            .bind(&base_sum)
            .execute(&mut *tx)
            .await?;
        ran.push("0001_schema.sql");
    }
    for (name, sql) in MIGRATIONS {
        match known(name) {
            Some(c) if c == sum(sql) => continue,
            Some(_) => tracing::warn!("migration {name} changed after it was applied; leaving it (add a new migration instead)"),
            None => {
                sqlx::raw_sql(sql).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO lumi_schema_migrations (name, checksum) VALUES ($1, $2)").bind(name).bind(sum(sql)).execute(&mut *tx).await?;
                ran.push(name);
            }
        }
    }
    tx.commit().await?;
    if ran.is_empty() {
        tracing::info!("schema up to date (no migrations to run)");
    } else {
        tracing::info!("schema migrated: {}", ran.join(", "));
    }
    Ok(())
}

/// SQL helpers the Rust queries use. `lumi_iso` renders a timestamp exactly
/// like Python's datetime.isoformat() (6-digit fraction, or none at .000000),
/// which the apps' fixed-format date parsers depend on.
pub async fn ensure_helpers(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        r#"CREATE OR REPLACE FUNCTION lumi_iso(t timestamp) RETURNS text LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $f$
             SELECT CASE WHEN t IS NULL THEN NULL
                         WHEN date_part('microseconds', t)::bigint % 1000000 = 0 THEN to_char(t, 'YYYY-MM-DD"T"HH24:MI:SS')
                         ELSE to_char(t, 'YYYY-MM-DD"T"HH24:MI:SS.US') END
           $f$;"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Python isoformat() of a naive timestamp.
pub fn py_iso(t: &chrono::NaiveDateTime) -> String {
    use chrono::Timelike;
    if t.nanosecond() / 1000 == 0 {
        t.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        t.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    }
}

/// Insert into ios_app_event_log; never fails the caller.
pub async fn log_event(pool: &PgPool, category: &str, event: &str, user_id: Option<&str>, level: &str, message: &str, detail: Option<Value>) {
    log_event_from(pool, "bridge", category, event, user_id, level, message, detail).await
}

#[allow(clippy::too_many_arguments)]
pub async fn log_event_from(pool: &PgPool, source: &str, category: &str, event: &str, user_id: Option<&str>, level: &str, message: &str, detail: Option<Value>) {
    let r = sqlx::query(
        "INSERT INTO ios_app_event_log (source, user_id, category, event, level, message, detail) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(truncate(source, 20))
    .bind(user_id.map(|u| truncate(u, 36)))
    .bind(truncate(category, 30))
    .bind(truncate(event, 60))
    .bind(truncate(if level.is_empty() { "info" } else { level }, 10))
    .bind(truncate(message, 2000))
    .bind(detail.map(sqlx::types::Json))
    .execute(pool)
    .await;
    if let Err(e) = r {
        tracing::error!("log_event failed (category={category} event={event}): {e}");
    }
}

/// Character-safe prefix, like Python's `s[:n]`.
pub fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
