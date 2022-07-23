use askama::Template;
use axum::extract::Path;
use axum::{
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use diesel_async::AsyncPgConnection;

use crate::models::{Drop, DropStatus, User};
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
