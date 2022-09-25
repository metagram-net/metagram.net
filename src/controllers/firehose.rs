use askama::Template;
use axum::response::{IntoResponse, Redirect};
use axum_extra::routing::TypedPath;
use serde::Deserialize;

use crate::models::User;
use crate::{Context, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose")]
pub struct Root;

pub async fn index(_: Root, session: Option<Session>) -> impl IntoResponse {
    match session {
        None => Redirect::to(&About.to_string()),
        Some(_) => Redirect::to(&super::streams::Member::path("unread")),
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/about")]
pub struct About;

#[derive(Template)]
#[template(path = "firehose/about.html")]
struct AboutPage {
    context: Context,
    user: Option<User>,
}

pub async fn about(_: About, context: Context, session: Option<Session>) -> impl IntoResponse {
    AboutPage {
        context,
        user: session.map(|s| s.user),
    }
}
