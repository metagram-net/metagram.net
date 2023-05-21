use askama::Template;
use axum::headers::HeaderName;
use axum::response::{IntoResponse, Redirect};
use axum::{Json, Router};
use axum_extra::routing::{RouterExt, TypedPath};
use http::header;
use serde::Deserialize;

use crate::models::User;
use crate::{AppState, Context, Session};

pub fn router() -> Router<AppState> {
    Router::new()
        .typed_get(index)
        .typed_get(about)
        .typed_get(manifest)
        .typed_get(service_worker)
}

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

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/manifest.json")]
pub struct Manifest;

pub mod pwa {
    use serde::Serialize;

    #[derive(Debug, Clone, Serialize)]
    pub struct Manifest {
        pub name: String,
        pub icons: Vec<Icon>,
        pub start_url: String,
        pub background_color: String,
        pub display: String,
        pub scope: String,
        pub theme_color: String,
        pub share_target: ShareTarget,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct Icon {
        pub src: String,
        pub r#type: String,
        pub sizes: String,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct ShareTarget {
        pub action: String,
        pub method: String,
        pub enctype: String,
        pub params: ShareParams,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct ShareParams {
        pub text: String,
        pub title: String,
        pub url: String,
    }
}

pub async fn manifest(_: Manifest) -> ([(HeaderName, &'static str); 1], Json<pwa::Manifest>) {
    let manifest = pwa::Manifest {
        name: "Firehose".to_string(),
        icons: vec![
            pwa::Icon {
                src: "/dist/images/firehose-192.png".to_string(),
                r#type: "image/png".to_string(),
                sizes: "192x192".to_string(),
            },
            pwa::Icon {
                src: "/dist/images/firehose-512.png".to_string(),
                r#type: "image/png".to_string(),
                sizes: "512x512".to_string(),
            },
        ],
        start_url: crate::controllers::streams::Member::path("unread"),
        background_color: "#C21B29".to_string(),
        display: "standalone".to_string(),
        // The trailing slash is required for the whole directory to be in-scope.
        scope: Root.to_string() + "/",
        theme_color: "#C21B29".to_string(),
        share_target: pwa::ShareTarget {
            action: crate::controllers::drops::New.to_string(),
            method: "GET".to_string(),
            enctype: "application/x-www-form-urlencoded".to_string(),
            params: pwa::ShareParams {
                text: "text".to_string(),
                title: "title".to_string(),
                url: "url".to_string(),
            },
        },
    };

    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        Json(manifest),
    )
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/service_worker.js")]
pub struct ServiceWorker;

pub async fn service_worker(_: ServiceWorker) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("service_worker.js"),
    )
}
