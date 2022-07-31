use askama::Template;
use axum::{response::IntoResponse, Json};
use axum_extra::routing::TypedPath;
use serde::{Deserialize, Serialize};

use crate::models::User;
use crate::{Context, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/")]
pub struct Root;

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    context: Context,
    user: Option<User>,
}

pub async fn index(_: Root, context: Context, session: Option<Session>) -> impl IntoResponse {
    Index {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/.well-known/health-check")]
pub struct HealthCheck;

#[derive(Serialize)]
struct Health {
    status: String,
}

pub async fn health_check(_: HealthCheck) -> impl IntoResponse {
    let health = Health {
        status: "Ok".to_string(),
    };
    Json(health)
}
