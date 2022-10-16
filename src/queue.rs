use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDateTime as Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::Acquire;
use sqlx::FromRow;
use sqlx::PgConnection;
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinError;
use uuid::Uuid;

type PgTransaction<'tx> = sqlx::Transaction<'tx, sqlx::Postgres>;

pub struct Worker {
    db: PgPool,
    interval: Duration,
}

impl Worker {
    pub fn new(db: PgPool, interval: Duration) -> Self {
        Self { db, interval }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), JoinError> {
        let mut interval = tokio::time::interval(self.interval);
        let mut found_job = true;

        tokio::spawn(async move {
            loop {
                // If the queue was empty last time we polled it (or polling resulted in an error),
                // wait a bit to avoid just constantly polling.
                if !found_job {
                    tracing::info!("Waiting before next poll");
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        _ = interval.tick() => (),
                    }
                }

                found_job = false;

                let mut tx = tokio::select! {
                    _ = shutdown.changed() => break,
                    res = self.db.begin() => match res {
                        Ok(tx) => tx,
                        Err(err) => {
                            tracing::error!({ ?err }, "Failed to open transaction");
                            continue;
                        }
                    }
                };

                let job = tokio::select! {
                    _ = shutdown.changed() => break,
                    res = claim_job(&mut *tx, chrono::Utc::now()) => match res {
                        Ok(Some(job)) => job,
                        Ok(None) => {
                            tracing::info!("No jobs to run");
                            continue;
                        }
                        Err(err) => {
                            tracing::error!({ ?err }, "Failed to claim job");
                            continue;
                        }
                    }
                };

                found_job = true;

                // Don't select! with the shutdown signal here. Jobs should be relatively
                // short-lived, so give this one a chance to complete before checking again.
                match Self::run_next_job(tx, job).await {
                    Ok(_) => (),
                    Err(err) => tracing::error!({ ?err }, "Worker failed to run job"),
                }
            }
        })
        .await
    }

    async fn run_next_job(mut tx: PgTransaction<'_>, job: Job) -> anyhow::Result<()> {
        tracing::info!({ ?job }, "Running job");

        let task: Box<dyn Task> = serde_json::from_value(job.params.clone())?;
        let mut ctx = Context { tx: &mut *tx };

        match task.run(&mut ctx).await {
            Ok(_) => {
                tracing::info!({ ?job }, "Job succeeded");
                mark_success(&mut *tx, job, chrono::Utc::now()).await?;
            }
            Err(err) => {
                tracing::error!({ ?job, ?err }, "Job failed");
                mark_failure(&mut *tx, job, chrono::Utc::now(), err.to_string()).await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }
}

#[typetag::serde(tag = "type")]
#[async_trait]
pub trait Task: Send + Sync {
    async fn run(&self, ctx: &mut Context) -> anyhow::Result<()>;
}

pub struct Context<'a> {
    pub tx: &'a mut PgConnection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Job {
    pub id: Uuid,
    pub params: serde_json::Value,
    pub scheduled_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub error: Option<String>,
}

pub async fn push(
    conn: &mut PgConnection,
    task: &dyn Task,
    scheduled_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Job> {
    insert_job(conn, task, scheduled_at).await
}

pub async fn push_uniq(
    conn: &mut PgConnection,
    task: &dyn Task,
    scheduled_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Job> {
    let mut tx = conn.begin().await?;

    let res = find_job_by_type(&mut *tx, task.typetag_name()).await?;
    let job = match res {
        Some(job) => job,
        None => insert_job(&mut *tx, task, scheduled_at).await?,
    };

    tx.commit().await?;
    Ok(job)
}

async fn find_job_by_type(conn: &mut PgConnection, type_name: &str) -> anyhow::Result<Option<Job>> {
    let res = sqlx::query_as!(
        Job,
        "
        select * from jobs
        where params->'type' = $1
        and finished_at is null
        ",
        serde_json::json!(type_name),
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(res)
}

async fn insert_job(
    conn: &mut PgConnection,
    task: &dyn Task,
    scheduled_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Job> {
    let params = serde_json::to_value(task)?;

    let job = sqlx::query_as!(
        Job,
        "
        insert into jobs
        (params, scheduled_at)
        values
        ($1, $2)
        returning *
        ",
        params,
        scheduled_at.naive_utc(),
    )
    .fetch_one(conn)
    .await?;
    Ok(job)
}

pub async fn claim_job(
    conn: &mut PgConnection,
    now: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Option<Job>> {
    sqlx::query_as!(
        Job,
        "
        update jobs
        set started_at = $1
        where id in (
            select id from jobs
            where started_at is null
            order by scheduled_at asc
            for update skip locked
            limit 1
        )
        returning *
        ",
        now.naive_utc(),
    )
    .fetch_optional(conn)
    .await
}

async fn mark_success(
    conn: &mut PgConnection,
    job: Job,
    now: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Job> {
    sqlx::query_as!(
        Job,
        "
        update jobs
        set finished_at = $1
        where id = $2
        returning *
        ",
        now.naive_utc(),
        job.id,
    )
    .fetch_one(conn)
    .await
}

async fn mark_failure(
    conn: &mut PgConnection,
    job: Job,
    now: chrono::DateTime<chrono::Utc>,
    error: String,
) -> sqlx::Result<Job> {
    sqlx::query_as!(
        Job,
        "
        update jobs
        set finished_at = $1
          , error = $2
        where id = $3
        returning *
        ",
        now.naive_utc(),
        error,
        job.id,
    )
    .fetch_one(conn)
    .await
}

pub async fn clear_finished(
    conn: &mut PgConnection,
    now: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Vec<Job>> {
    sqlx::query_as!(
        Job,
        "
        delete from jobs
        where finished_at < $1
        and error is null
        returning *
        ",
        now.naive_utc(),
    )
    .fetch_all(conn)
    .await
}
