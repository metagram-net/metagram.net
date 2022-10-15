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
#[template(path = "home/index.html")]
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

#[derive(TypedPath, Deserialize)]
#[typed_path("/about")]
pub struct About;

#[derive(Template)]
#[template(path = "home/about.html")]
struct AboutPage {
    context: Context,
    user: Option<User>,

    crate_version: String,
    build_info: String,
    source_url: String,
}

pub async fn about(_: About, context: Context, session: Option<Session>) -> impl IntoResponse {
    AboutPage {
        context,
        user: session.map(|s| s.user),
        crate_version: format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        build_info: crate::BUILD_INFO.to_string(),
        source_url: crate::SOURCE_URL.to_string(),
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/licenses")]
pub struct Licenses;

#[derive(Template)]
#[template(path = "home/licenses.html")]
struct LicensesPage {
    context: Context,
    user: Option<User>,
}

pub async fn licenses(
    _: Licenses,
    context: Context,
    session: Option<Session>,
) -> impl IntoResponse {
    LicensesPage {
        context,
        user: session.map(|s| s.user),
    }
}
