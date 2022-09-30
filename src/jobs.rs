use fang::asynk::async_queue::AsyncQueueable;
use fang::serde::{Deserialize, Serialize};
use fang::{async_trait, Scheduled};
use fang::{AsyncRunnable, FangError};

// Reminder: This cron syntax is different than most cron expressions.
//
//   sec  min   hour   day of month   month   day of week   year
//     0    1      2              3       4             5      6
//
//

#[derive(Serialize, Deserialize)]
pub struct Tick {}

#[typetag::serde]
#[async_trait]
impl AsyncRunnable for Tick {
    async fn run(&self, _queueable: &mut dyn AsyncQueueable) -> Result<(), FangError> {
        tracing::info!("Tick!");
        Ok(())
    }

    fn uniq(&self) -> bool {
        true
    }

    fn cron(&self) -> Option<fang::Scheduled> {
        Some(Scheduled::CronPattern("0 * * * * * *".to_string()))
    }
}
