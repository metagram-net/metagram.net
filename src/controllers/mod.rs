use axum::Router;

use crate::AppState;

pub mod auth;
pub mod drops;
pub mod errors;
pub mod firehose;
pub mod home;
pub mod hydrants;
pub mod streams;
pub mod tags;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(drops::router())
        .merge(errors::router())
        .merge(firehose::router())
        .merge(home::router())
        .merge(hydrants::router())
        .merge(streams::router())
        .merge(tags::router())
}
