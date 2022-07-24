use askama::Template;
use axum::extract::Path;
use axum::{
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use diesel_async::AsyncPgConnection;

use crate::models::{Drop, DropStatus, NewDrop, User};
use crate::{schema, Context, PgConn, Session};

pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/about", get(about))
        .route("/streams/:id", get(stream))
}

async fn index(session: Option<Session>) -> impl IntoResponse {
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

async fn about(context: Context, session: Option<Session>) -> impl IntoResponse {
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

async fn stream(
    context: Context,
    session: Session,
    Path(id): Path<String>,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    match id.as_str() {
        "unread" => {
            let rows = list_drops(&mut db, session.user.clone(), DropStatus::Unread).await;
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

async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    v_status: DropStatus,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::status.eq(v_status)))
        .load(db)
        .await?;
    Ok(res)
}

#[derive(Default)]
pub struct DropFields {
    pub title: Option<String>,
    pub url: String,
    pub status: Option<DropStatus>,
}

pub async fn create_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    attrs: DropFields,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: Drop = insert_into(t::drops)
        .values(&NewDrop {
            user_id: user.id,
            title: attrs.title.as_ref().map(|x| x as _),
            url: &attrs.url,
            status: attrs.status.unwrap_or(DropStatus::Unread),
            moved_at: now.naive_utc(),
        })
        .get_result(db)
        .await?;
    Ok(drop)
}
