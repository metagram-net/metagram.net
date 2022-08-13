use askama::Template;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use uuid::Uuid;

use crate::firehose;
use crate::models::{DropStatus, User};
use crate::{Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/streams")]
pub struct Collection;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/streams/new")]
pub struct New;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/streams/:id")]
pub struct Member {
    id: String,
}

impl Member {
    pub fn path(id: &str) -> String {
        Self { id: id.to_string() }.to_string()
    }

    pub fn path_uuid(id: &Uuid) -> String {
        Self { id: id.to_string() }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/streams/:id/edit")]
pub struct Edit {
    id: String,
}

#[derive(Template)]
#[template(path = "firehose/streams/index.html")]
struct Index {
    context: Context,
    user: Option<User>,
    streams: Vec<firehose::Stream>,
}

pub async fn index(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let streams = match firehose::list_streams(&mut db, &session.user).await {
        Ok(streams) => streams,
        Err(err) => return Err(context.error(Some(session), err.into())),
    };

    Ok(Index {
        context,
        user: Some(session.user),
        streams,
    })
}

#[derive(Template)]
#[template(path = "firehose/streams/show.html")]
struct ShowPage {
    context: Context,
    user: Option<User>,
    stream: firehose::Stream,
    drops: Vec<firehose::Drop>,
}

pub async fn show(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let stream: anyhow::Result<firehose::Stream> = match id.as_str() {
        "unread" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Unread,
        })),
        "read" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Read,
        })),
        "saved" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Saved,
        })),

        id => match Uuid::parse_str(id) {
            Ok(id) => firehose::find_stream(&mut db, &session.user, id)
                .await
                .map(firehose::Stream::Custom),
            Err(err) => Err(err.into()),
        },
    };

    let stream = match stream {
        Ok(stream) => stream,
        Err(err) => {
            tracing::error!({ ?err, ?session.user.id, ?id }, "Stream not found");
            return Err(StatusCode::NOT_FOUND.into_response());
        }
    };

    let drops = firehose::list_drops(&mut db, session.user.clone(), stream.filters()).await;

    match drops {
        Ok(drops) => Ok(ShowPage {
            context,
            user: Some(session.user),
            stream,
            drops,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}
