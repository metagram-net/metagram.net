use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinError;
use uuid::Uuid;

use crate::queue::{Context, Task};
use crate::{firehose, queue};

pub async fn cron(db: PgPool, mut shutdown: watch::Receiver<bool>) -> Result<(), JoinError> {
    let mut hourly = tokio::time::interval(Duration::from_secs(60 * 60));

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = hourly.tick() => {
                    match push_cron(&db).await {
                        Ok(_) => tracing::info!("Scheduled cron"),
                        Err(err) => tracing::error!({ ?err }, "Failed to schedule cron"),
                    }
                },
            }
        }
    })
    .await
}

async fn push_cron(pool: &PgPool) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    queue::push_uniq(&mut conn, &HydrateAll {}, chrono::Utc::now()).await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tick {}

#[typetag::serde]
#[async_trait]
impl Task for Tick {
    async fn run(&self, _ctx: &mut Context) -> anyhow::Result<()> {
        tracing::info!("Tick!");
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HydrateAll {}

#[typetag::serde]
#[async_trait]
impl Task for HydrateAll {
    async fn run(&self, ctx: &mut Context) -> anyhow::Result<()> {
        let now = chrono::Utc::now();

        let stale = firehose::stale_hydrants(&mut *ctx.tx, now).await?;

        for hydrant in stale {
            let task = HydrateOne {
                hydrant_id: hydrant.hydrant.id,
            };
            queue::push(&mut *ctx.tx, &task, now).await?;
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HydrateOne {
    hydrant_id: Uuid,
}

#[typetag::serde]
#[async_trait]
impl Task for HydrateOne {
    async fn run(&self, ctx: &mut Context) -> anyhow::Result<()> {
        let now = chrono::Utc::now();
        let client = reqwest::Client::new();

        firehose::Hydrant::fetch(&mut *ctx.tx, &client, self.hydrant_id, now).await
    }
}
