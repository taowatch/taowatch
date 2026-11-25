use anyhow::Result;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Connect to Postgres and apply pending migrations from `backend/migrations`.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new().max_connections(10).connect(database_url).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}
