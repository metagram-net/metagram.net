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
    let filters = match id.as_str() {
        "unread" => firehose::DropFilters {
            status: Some(DropStatus::Unread),
            ..Default::default()
        },

        "read" => firehose::DropFilters {
            status: Some(DropStatus::Read),
            ..Default::default()
        },

        "saved" => firehose::DropFilters {
            status: Some(DropStatus::Saved),
            ..Default::default()
        },

        _id => todo!("feat: custom streams"),
    };

    let drops = firehose::list_drops(&mut db, session.user.clone(), filters).await;

    match drops {
        Ok(drops) => Ok(ShowPage {
            context,
            user: Some(session.user),
            drops,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}
