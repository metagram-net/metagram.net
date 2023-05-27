use askama::Template;
use axum::response::{IntoResponse, Redirect};
use axum::Router;
use axum_extra::routing::RouterExt;
use axum_extra::{extract::Form, routing::TypedPath};
use serde::Deserialize;
use uuid::Uuid;

use crate::filters;
use crate::firehose;
use crate::models::{DropStatus, User};
use crate::view_models::{tag_options, TagOption};
use crate::{AppState, Context, PgConn, Session};

pub fn router() -> Router<AppState> {
    Router::new()
        .typed_get(index)
        .typed_get(new)
        .typed_post(create)
        .typed_get(show)
        .typed_get(edit)
        .typed_post(update)
}

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
    id: Uuid,
}

impl Edit {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
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
) -> super::Result<impl IntoResponse> {
    let streams = firehose::list_streams(&mut db, &session.user).await?;

    Ok(Index {
        context,
        user: Some(session.user),
        streams,
    })
}

#[derive(Template)]
#[template(path = "firehose/streams/new.html")]
struct NewStream {
    context: Context,
    user: Option<User>,
    stream: StreamForm,
    tag_options: Vec<TagOption>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct StreamForm {
    name: String,
    tags: Vec<String>,

    authenticity_token: String,
    errors: Option<Vec<String>>,
}

impl StreamForm {
    fn validate(&self) -> Result<(), Vec<String>> {
        if let Some(errors) = &self.errors {
            return Err(errors.to_vec());
        }

        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("Name cannot be blank".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl From<firehose::CustomStream> for StreamForm {
    fn from(stream: firehose::CustomStream) -> Self {
        StreamForm {
            name: stream.stream.name,
            tags: stream
                .tags
                .iter()
                .cloned()
                .map(|t| t.id.to_string())
                .collect(),

            ..Default::default()
        }
    }
}

pub async fn new(
    _: New,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> super::Result<impl IntoResponse> {
    let tags = firehose::list_tags(&mut db, &session.user).await?;

    Ok(NewStream {
        context,
        user: Some(session.user),
        stream: Default::default(),
        tag_options: tag_options(tags),
    })
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(mut form): Form<StreamForm>,
) -> super::Result<impl IntoResponse> {
    context.verify_csrf(&form.authenticity_token)?;
    form.errors = form.validate().err();

    let tag_ids: Vec<Uuid> = form
        .tags
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    if tag_ids.len() != form.tags.len() {
        tracing::warn!({ ?form.tags, ?tag_ids }, "Some tags could not be found");

        form.errors = {
            let mut errs = form.errors.unwrap_or_default();
            errs.push("Error finding tags. Was one recently deleted?".to_string());
            Some(errs)
        };

        let tags = firehose::list_tags(&mut db, &session.user).await?;

        return Ok(NewStream {
            context,
            user: Some(session.user),
            stream: form,
            tag_options: tag_options(tags),
        }
        .into_response());
    }

    let tags = firehose::find_tags(&mut db, &session.user, &tag_ids).await?;

    match firehose::create_stream(&mut db, &session.user, &form.name, &tags).await {
        Ok(stream) => Ok(Redirect::to(
            &Member {
                id: stream.stream.id.to_string(),
            }
            .to_string(),
        )
        .into_response()),
        Err(err) => {
            tracing::error!({ ?err }, "could not create stream");

            let tags = firehose::list_tags(&mut db, &session.user).await?;

            Ok(NewStream {
                context,
                user: Some(session.user),
                stream: form,
                tag_options: tag_options(tags),
            }
            .into_response())
        }
    }
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
) -> super::Result<impl IntoResponse> {
    let stream: firehose::Stream = match id.as_str() {
        "unread" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Unread,
        })),
        "read" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Read,
        })),
        "saved" => Ok(firehose::Stream::Status(firehose::StatusStream {
            status: DropStatus::Saved,
        })),

        id => firehose::find_stream(&mut db, &session.user, parse_stream_id(id)?)
            .await
            .map(firehose::Stream::Custom),
    }?;

    let mut filters = stream.filters();

    // Custom streams don't have a default status filter, so fill one in.
    if filters.status.is_none() {
        filters.status = Some(DropStatus::Unread);
    }

    let drops = firehose::list_drops(&mut db, &session.user, filters).await?;

    Ok(ShowPage {
        context,
        user: Some(session.user),
        stream,
        drops,
    })
}

#[derive(Template)]
#[template(path = "firehose/streams/edit.html")]
struct EditStream {
    context: Context,
    user: Option<User>,
    id: Uuid,
    stream: StreamForm,
    tag_options: Vec<TagOption>,
}

pub async fn edit(
    Edit { id }: Edit,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> super::Result<impl IntoResponse> {
    let stream = firehose::find_stream(&mut db, &session.user, id).await?;
    let tags = firehose::list_tags(&mut db, &session.user).await?;

    Ok(EditStream {
        context,
        user: Some(session.user),
        id,
        stream: stream.into(),
        tag_options: tag_options(tags),
    })
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(mut form): Form<StreamForm>,
) -> super::Result<impl IntoResponse> {
    context.verify_csrf(&form.authenticity_token)?;
    form.errors = form.validate().err();

    let id = parse_stream_id(&id)?;

    let stream = firehose::find_stream(&mut db, &session.user, id).await?;

    let tag_ids: Vec<Uuid> = form
        .tags
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    if tag_ids.len() != form.tags.len() {
        tracing::warn!({ ?form.tags, ?tag_ids }, "Some tags could not be found");

        form.errors = {
            let mut errs = form.errors.unwrap_or_default();
            errs.push("Error finding tags. Was one recently deleted?".to_string());
            Some(errs)
        };

        let tags = firehose::list_tags(&mut db, &session.user).await?;

        return Ok(EditStream {
            context,
            user: Some(session.user),
            id,
            stream: form,
            tag_options: tag_options(tags),
        }
        .into_response());
    }

    let tags = firehose::find_tags(&mut db, &session.user, &tag_ids).await?;

    let fields = firehose::StreamFields {
        name: Some(form.name.clone()),
        tag_ids: Some(tags.iter().map(|t| t.id).collect()),
    };

    let stream = firehose::update_stream(&mut db, &session.user, &stream.stream, fields).await;
    match stream {
        Ok(stream) => Ok(Redirect::to(
            &Member {
                id: stream.stream.id.to_string(),
            }
            .to_string(),
        )
        .into_response()),
        Err(err) => {
            tracing::error!({ ?err }, "could not update stream");

            let tags = firehose::list_tags(&mut db, &session.user).await?;

            Ok(EditStream {
                context,
                user: Some(session.user),
                id,
                stream: form,
                tag_options: tag_options(tags),
            }
            .into_response())
        }
    }
}

fn parse_stream_id(id: &str) -> super::Result<Uuid> {
    Uuid::parse_str(id).map_err(|_| super::Error::StreamNotFound {
        stream_id: id.to_string(),
    })
}
