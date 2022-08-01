use askama::Template;
use axum::response::{IntoResponse, Response};
use axum_extra::routing::TypedPath;
use serde::Deserialize;

use crate::firehose;
use crate::models::{DropStatus, User};
use crate::{Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/streams/:id")]
pub struct Member {
    id: String,
}

impl Member {
    pub fn path(id: &str) -> String {
        Self { id: id.to_string() }.to_string()
    }
}

#[derive(Template)]
#[template(path = "firehose/stream.html")]
struct ShowPage {
    context: Context,
    user: Option<User>,
    drops: Vec<firehose::Drop>,
}

pub async fn show(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let drops: anyhow::Result<Vec<firehose::Drop>> = match id.as_str() {
        "unread" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Unread).await,
        "read" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Read).await,
        "saved" => firehose::list_drops(&mut db, session.user.clone(), DropStatus::Saved).await,
        _id => todo!("feat: custom streams"),
    };

    match drops {
        Ok(drops) => Ok(ShowPage {
            context,
            user: Some(session.user),
            drops,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}
