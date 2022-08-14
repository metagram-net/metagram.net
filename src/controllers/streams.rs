use askama::Template;
use axum::extract::Form;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use uuid::Uuid;

use crate::filters;
use crate::firehose;
use crate::models::{DropStatus, User};
use crate::view_models::{tag_options, TagOption};
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
#[template(path = "firehose/streams/new.html")]
struct NewStream {
    context: Context,
    user: Option<User>,
    stream: StreamForm,
    tag_options: Vec<TagOption>,
}

#[derive(Default, Deserialize)]
pub struct StreamForm {
    name: String,
    tags: Vec<String>,

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

pub async fn new(
    _: New,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let tags = match firehose::list_tags(&mut db, &session.user).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

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
) -> Result<Redirect, impl IntoResponse> {
    let errors = match form.validate() {
        Ok(_) => None,
        Err(errors) => Some(errors),
    };
    form.errors = errors;

    let tag_ids: Vec<Uuid> = form
        .tags
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    if tag_ids.len() != form.tags.len() {
        tracing::warn!({ ?form.tags }, "Some tags could not be found");
        // TODO: this should re-render the form
    }

    let tags = match firehose::find_tags(&mut db, &session.user, &tag_ids).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    match firehose::create_stream(&mut db, &session.user, &form.name, &tags).await {
        Ok(stream) => Ok(Redirect::to(
            &Member {
                id: stream.stream.id.to_string(),
            }
            .to_string(),
        )),
        Err(err) => {
            tracing::error!({ ?err }, "could not create stream");

            let tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(NewStream {
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
) -> Result<impl IntoResponse, Response> {
    let stream = match firehose::find_stream(&mut db, &session.user, id).await {
        Ok(stream) => stream,
        Err(_) => return Err(StatusCode::NOT_FOUND.into_response()),
    };

    let tags = match firehose::list_tags(&mut db, &session.user).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    Ok(EditStream {
        context,
        user: Some(session.user),
        id,
        stream: StreamForm {
            name: stream.stream.name,
            errors: None,
            tags: stream
                .tags
                .iter()
                .cloned()
                .map(|t| t.id.to_string())
                .collect(),
        },
        tag_options: tag_options(tags),
    })
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(form): Form<StreamForm>,
) -> Result<Redirect, Response> {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return Err(StatusCode::NOT_FOUND.into_response()),
    };

    let stream = match firehose::find_stream(&mut db, &session.user, id).await {
        Ok(stream) => stream,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    let tag_ids: Vec<Uuid> = form
        .tags
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    if tag_ids.len() != form.tags.len() {
        tracing::warn!({ ?form.tags }, "Some tags could not be found");
        // TODO: this should re-render the form
    }

    let tags = match firehose::find_tags(&mut db, &session.user, &tag_ids).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

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
        )),
        Err(err) => {
            tracing::error!({ ?err }, "could not update stream");

            let tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(EditStream {
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
