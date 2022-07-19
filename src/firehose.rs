use super::{Context, Session, User}; // TODO: mod auth
use askama::Template;
use axum::extract::Path;
use axum::{
    extract::Extension,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use sqlx::postgres::PgPool;

pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/about", get(about))
        .route("/streams/:id", get(stream))
}

pub async fn index(session: Option<Session>) -> impl IntoResponse {
    match session {
        None => Redirect::to("/firehose/about"),
        Some(_) => Redirect::to("/firehose/streams/unread"),
    }
}

#[derive(Template)]
#[template(path = "firehose/about.html")]
struct About {
    context: Context,
    user: Option<User>,
}

pub async fn about(context: Context, session: Option<Session>) -> impl IntoResponse {
    About {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(Template)]
#[template(path = "firehose/status_stream.html")]
struct StatusStream {
    context: Context,
    user: Option<User>,
    drops: Vec<Drop>,
}

pub async fn stream(
    context: Context,
    session: Session,
    Path(id): Path<String>,
    Extension(pool): Extension<PgPool>,
) -> Result<impl IntoResponse, Response> {
    match id.as_str() {
        "unread" => {
            let rows = list_drops(&pool, session.user.clone(), DropStatus::Unread).await;
            let drops = match rows {
                Ok(drops) => drops,
                Err(err) => return Err(context.error(Some(session), err.into())),
            };
            Ok(StatusStream {
                context,
                user: Some(session.user),
                drops,
            })
        }
        "read" => todo!(),
        "saved" => todo!(),
        _id => todo!("feat: custom stream IDs"),
    }
}

#[derive(Debug, Clone, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "snake_case")]
enum DropStatus {
    Unread,
    Read,
    Saved,
}

impl std::fmt::Display for DropStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Self::Unread => "unread",
            Self::Read => "read",
            Self::Saved => "saved",
        };
        write!(f, "{}", s)
    }
}

struct InvalidStatusError(String);

impl std::str::FromStr for DropStatus {
    type Err = InvalidStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unread" => Ok(Self::Unread),
            "read" => Ok(Self::Read),
            "saved" => Ok(Self::Saved),
            s => Err(InvalidStatusError(s.to_string())),
        }
    }
}

#[allow(unused)]
#[derive(Debug, Clone, sqlx::FromRow)]
struct Drop {
    id: uuid::Uuid,
    title: Option<String>,      // TODO(v0.2): make non-nullable
    status: Option<DropStatus>, // TODO(v0.2): make non-nullable
    moved_at: chrono::NaiveDateTime,
    article_id: uuid::Uuid,
    user_id: uuid::Uuid,

    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
}

impl Drop {
    fn url(&self) -> String {
        "https://example.com/TODO".to_string() // TODO: Load from article
    }

    fn domain(&self) -> String {
        "example.com".to_string() // TODO: Do the PSL thing
    }

    fn display_text(&self) -> String {
        self.title.as_ref().unwrap_or(&self.url()).to_string()
    }
}

async fn list_drops(pool: &PgPool, user: User, status: DropStatus) -> anyhow::Result<Vec<Drop>> {
    let drops: Vec<Drop> = sqlx::query_as("select * from drops where user_id = $1 and status = $2")
        .bind(user.id)
        .bind(status.to_string())
        .fetch_all(pool)
        .await?;
    Ok(drops)
}
