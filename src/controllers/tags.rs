use askama::Template;
use axum::{
    extract::Form,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::filters;
use crate::firehose::{self, DropStatus};
use crate::models::{Tag, User};
use crate::{Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/tags")]
pub struct Collection;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/tags/new")]
pub struct New;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/tags/:id")]
pub struct Member {
    id: Uuid,
}

impl Member {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/tags/:id/edit")]
pub struct Edit {
    id: Uuid,
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/tags/:id/move")]
pub struct Move {
    id: Uuid,
}

#[derive(Template)]
#[template(path = "firehose/tags/index.html")]
struct Index {
    context: Context,
    user: Option<User>,
    tags: Vec<Tag>,
}

pub async fn index(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut conn): PgConn,
) -> Result<impl IntoResponse, Response> {
    let tags = firehose::list_tags(&mut conn, &session.user).await;

    match tags {
        Ok(tags) => Ok(Index {
            context,
            user: Some(session.user),
            tags,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct TagForm {
    name: String,
    color: String,

    errors: Option<Vec<String>>,
}

impl TagForm {
    fn validate(&self) -> Result<(), Vec<String>> {
        use lazy_static::lazy_static;
        use regex::Regex;

        if let Some(errors) = &self.errors {
            return Err(errors.to_vec());
        }

        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("Name cannot be blank".to_string());
        }
        lazy_static! {
            static ref RE_COLOR: Regex = Regex::new("^#[0-9a-fA-F]{6}$").unwrap();
        }
        if !RE_COLOR.is_match(&self.color) {
            errors.push(format!("Invalid color code: {}", self.color));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/tags/new.html")]
struct NewTag {
    context: Context,
    user: Option<User>,
    tag: TagForm,
}

pub async fn new(_: New, context: Context, session: Session) -> impl IntoResponse {
    NewTag {
        context,
        user: Some(session.user),
        tag: Default::default(),
    }
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut conn): PgConn,
    Form(mut form): Form<TagForm>,
) -> Result<Redirect, impl IntoResponse> {
    let errors = match form.validate() {
        Ok(_) => None,
        Err(errors) => Some(errors),
    };
    form.errors = errors;

    let tag = firehose::create_tag(&mut conn, &session.user, &form.name, &form.color).await;
    match tag {
        Ok(tag) => Ok(Redirect::to(&Member { id: tag.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not create tag");
            Err(NewTag {
                context,
                user: Some(session.user),
                tag: form,
            })
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/tags/show.html")]
struct Show {
    context: Context,
    user: Option<User>,
    tag: Tag,

    unread_drops: Vec<firehose::Drop>,
    read_drops: Vec<firehose::Drop>,
    saved_drops: Vec<firehose::Drop>,
}

pub async fn show(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut conn): PgConn,
) -> Result<impl IntoResponse, Response> {
    let tag = match firehose::find_tag(&mut conn, &session.user, id).await {
        Ok(tag) => tag,
        Err(err) => return Err(context.error(Some(session), err.into())),
    };

    let drops = match load_tag_drops(&mut conn, &session.user, tag.clone()).await {
        Ok(drops) => drops,
        Err(err) => return Err(context.error(Some(session), err.into())),
    };

    Ok(Show {
        context,
        user: Some(session.user),
        tag,
        unread_drops: drops.unread_drops,
        read_drops: drops.read_drops,
        saved_drops: drops.saved_drops,
    })
}

struct TagDrops {
    unread_drops: Vec<firehose::Drop>,
    read_drops: Vec<firehose::Drop>,
    saved_drops: Vec<firehose::Drop>,
}

async fn load_tag_drops(
    conn: &mut PgConnection,
    user: &User,
    tag: Tag,
) -> anyhow::Result<TagDrops> {
    let unread_drops = firehose::list_drops(
        &mut *conn,
        user,
        firehose::DropFilters {
            tags: Some(vec![tag.clone()]),
            status: Some(firehose::DropStatus::Unread),
        },
    )
    .await?;

    let read_drops = firehose::list_drops(
        &mut *conn,
        user,
        firehose::DropFilters {
            tags: Some(vec![tag.clone()]),
            status: Some(firehose::DropStatus::Read),
        },
    )
    .await?;

    let saved_drops = firehose::list_drops(
        &mut *conn,
        user,
        firehose::DropFilters {
            tags: Some(vec![tag.clone()]),
            status: Some(firehose::DropStatus::Saved),
        },
    )
    .await?;

    Ok(TagDrops {
        unread_drops,
        read_drops,
        saved_drops,
    })
}

#[derive(Template)]
#[template(path = "firehose/tags/edit.html")]
struct EditTag {
    context: Context,
    user: Option<User>,
    id: Uuid,
    tag: TagForm,
}

pub async fn edit(
    Edit { id }: Edit,
    context: Context,
    session: Session,
    PgConn(mut conn): PgConn,
) -> Result<impl IntoResponse, Response> {
    let tag = firehose::find_tag(&mut conn, &session.user, id).await;
    match tag {
        Ok(tag) => Ok(EditTag {
            context,
            user: Some(session.user),
            id,
            tag: TagForm {
                name: tag.name,
                color: tag.color,
                errors: None,
            },
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut conn): PgConn,
    Form(form): Form<TagForm>,
) -> Result<Redirect, Response> {
    let tag = match firehose::find_tag(&mut conn, &session.user, id).await {
        Ok(tag) => tag,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    let fields = firehose::TagFields {
        name: Some(form.name.clone()),
        color: Some(form.color.clone()),
    };

    let tag = firehose::update_tag(&mut conn, &session.user, tag, fields).await;
    match tag {
        Ok(tag) => Ok(Redirect::to(&Member { id: tag.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not update tag");
            Err(EditTag {
                context,
                user: Some(session.user),
                id,
                tag: form,
            }
            .into_response())
        }
    }
}
