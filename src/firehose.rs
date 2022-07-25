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
        .route("/drops", get(drops_index)) // TODO: The rest of the REST
        .route("/drops/:id", get(drops_show))
}

async fn drops_index() -> Redirect {
    Redirect::to("/firehose/streams/unread")
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
#[template(path = "firehose/stream.html")]
struct ShowStream {
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
    let drops: anyhow::Result<Vec<Drop>> = match id.as_str() {
        "unread" => list_drops(&mut db, session.user.clone(), DropStatus::Unread).await,
        "read" => list_drops(&mut db, session.user.clone(), DropStatus::Read).await,
        "saved" => list_drops(&mut db, session.user.clone(), DropStatus::Saved).await,
        _id => todo!("feat: custom streams"),
    };

    match drops {
        Ok(drops) => Ok(ShowStream {
            context,
            user: Some(session.user),
            drops,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    status: DropStatus,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::status.eq(status)))
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
            title: attrs.title.as_deref(),
            url: &attrs.url,
            status: attrs.status.unwrap_or(DropStatus::Unread),
            moved_at: now.naive_utc(),
        })
        .get_result(db)
        .await?;
    Ok(drop)
}

#[derive(Template)]
#[template(path = "firehose/drop.html")]
struct ShowDrop {
    context: Context,
    user: Option<User>,
    drop: Drop,
}

async fn drops_show(
    context: Context,
    session: Session,
    Path(id): Path<uuid::Uuid>,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    match find_drop(&mut db, &session.user, id).await {
        Ok(drop) => Ok(ShowDrop {
            context,
            user: Some(session.user),
            drop,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

pub async fn find_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    id: uuid::Uuid,
) -> anyhow::Result<Drop> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::id.eq(id)))
        .get_result(db)
        .await?;
    Ok(res)
}
