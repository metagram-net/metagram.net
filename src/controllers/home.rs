use askama::Template;
use axum::{response::IntoResponse, Json};
use serde::Serialize;

use crate::models::User;
use crate::{Context, Session};

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    context: Context,
    user: Option<User>,
}

pub async fn index(context: Context, session: Option<Session>) -> impl IntoResponse {
    Index {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(Serialize)]
struct Health {
    status: String,
}

pub async fn health_check() -> impl IntoResponse {
    let health = Health {
        status: "Ok".to_string(),
    };
    Json(health)
}
