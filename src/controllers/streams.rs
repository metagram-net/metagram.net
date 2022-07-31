use askama::Template;
use axum::extract::Path;
use axum::response::{IntoResponse, Response};

use crate::firehose;
use crate::models::{Drop, DropStatus, User};
use crate::{Context, PgConn, Session};

#[derive(Template)]
#[template(path = "firehose/stream.html")]
struct ShowStream {
    context: Context,
    user: Option<User>,
    drops: Vec<Drop>,
}

pub async fn show(
    context: Context,
    session: Session,
    Path(id): Path<String>,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let drops: anyhow::Result<Vec<Drop>> = match id.as_str() {
        "unread" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Unread).await,
        "read" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Read).await,
        "saved" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Saved).await,
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
