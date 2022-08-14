use std::collections::HashSet;

use askama::Template;
use axum::{
    extract::Form,
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use uuid::Uuid;

use crate::filters;
use crate::firehose;
use crate::models::{DropStatus, Tag, User};
use crate::{BaseUrl, Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops")]
pub struct Collection;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/new")]
pub struct New;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/:id")]
pub struct Member {
    id: Uuid,
}

// TODO: Is there a good way to derive path()?

impl Member {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/:id/edit")]
pub struct Edit {
    id: Uuid,
}

impl Edit {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/:id/move")]
pub struct Move {
    id: Uuid,
}

impl Move {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

pub async fn index(_: Collection) -> Redirect {
    Redirect::to("/firehose/streams/unread")
}

#[derive(Default, Deserialize)]
pub struct DropForm {
    title: String,
    url: String,
    tags: HashSet<String>,

    errors: Option<Vec<String>>,
}

impl DropForm {
    fn validate(&self) -> Result<(), Vec<String>> {
        if let Some(errors) = &self.errors {
            return Err(errors.to_vec());
        }

        let mut errors = Vec::new();
        if self.url.is_empty() {
            errors.push("URL cannot be blank".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/drops/new.html")]
struct NewDrop {
    context: Context,
    user: Option<User>,
    drop: DropForm,
    bookmarklet: String,
    tag_options: Vec<TagOption>,
}

// TODO: dedupe with streams controller
struct TagOption {
    id: String,
    name: String,
    color: String,
}

fn tag_options(tags: Vec<Tag>) -> Vec<TagOption> {
    tags.iter()
        .cloned()
        .map(|t| TagOption {
            id: t.id.to_string(),
            name: t.name,
            color: t.color,
        })
        .collect()
}

fn tag_selectors(opts: &HashSet<String>) -> Vec<firehose::TagSelector> {
    opts.iter()
        // Keep this prefix synced with the select2 options.
        .filter_map(|value| match value.strip_prefix('_') {
            Some(name) => Some(firehose::TagSelector::Create {
                name: name.to_string(),
                color: Tag::DEFAULT_COLOR.to_string(),
            }),
            None => match Uuid::parse_str(value) {
                Ok(id) => Some(firehose::TagSelector::Find { id }),
                Err(_) => {
                    // Well this is weird. There's probably a bug somewhere!
                    tracing::error!( { ?value }, "Could not interpret tag selector" );
                    None
                }
            },
        })
        .collect()
}

pub async fn new(
    _: New,
    Extension(base_url): Extension<BaseUrl>,
    PgConn(mut db): PgConn,
    context: Context,
    session: Session,
) -> Result<impl IntoResponse, Response> {
    let tags = match firehose::list_tags(&mut db, &session.user).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    Ok(NewDrop {
        context,
        user: Some(session.user),
        drop: Default::default(),
        bookmarklet: bookmarklet(base_url.0),
        tag_options: tag_options(tags),
    })
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    Extension(base_url): Extension<BaseUrl>,
    PgConn(mut db): PgConn,
    Form(mut form): Form<DropForm>,
) -> Result<Redirect, Response> {
    let now = chrono::Utc::now();
    let errors = match form.validate() {
        Ok(_) => None,
        Err(errors) => Some(errors),
    };
    form.errors = errors;

    // If the title is an empty string, set it to null instead.
    let title = coerce_empty(form.title.clone());

    let drop = firehose::create_drop(
        &mut db,
        session.user.clone(),
        title,
        form.url.clone(),
        Some(tag_selectors(&form.tags)),
        now,
    )
    .await;

    match drop {
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.drop.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not create drop");

            let all_tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(NewDrop {
                context,
                user: Some(session.user),
                drop: form,
                bookmarklet: bookmarklet(base_url.0),
                tag_options: tag_options(all_tags),
            }
            .into_response())
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/drops/show.html")]
struct Show {
    context: Context,
    user: Option<User>,
    drop: firehose::Drop,
}

pub async fn show(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    match firehose::find_drop(&mut db, &session.user, id).await {
        Ok(drop) => Ok(Show {
            context,
            user: Some(session.user),
            drop,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

#[derive(Template)]
#[template(path = "firehose/drops/edit.html")]
struct EditDrop {
    context: Context,
    user: Option<User>,
    id: Uuid,
    drop: DropForm,
    tag_options: Vec<TagOption>,
}

pub async fn edit(
    Edit { id }: Edit,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let drop = firehose::find_drop(&mut db, &session.user, id).await;

    match drop {
        Ok(drop) => {
            let selected_tags: HashSet<String> =
                drop.tags.iter().map(|t| t.id.to_string()).collect();

            let all_tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Ok(EditDrop {
                context,
                user: Some(session.user),
                id,
                drop: DropForm {
                    title: drop.drop.title.unwrap_or_default(),
                    url: drop.drop.url,
                    errors: None,
                    tags: selected_tags,
                },
                tag_options: tag_options(all_tags),
            })
        }
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(form): Form<DropForm>,
) -> Result<Redirect, Response> {
    let drop = match firehose::find_drop(&mut db, &session.user, id).await {
        Ok(drop) => drop,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    let fields = firehose::DropFields {
        title: coerce_empty(form.title.clone()),
        url: coerce_empty(form.url.clone()),
    };
    let tags = tag_selectors(&form.tags);

    let drop = firehose::update_drop(&mut db, session.user.clone(), drop, fields, Some(tags)).await;
    match drop {
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.drop.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not update drop");

            let all_tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(EditDrop {
                context,
                user: Some(session.user),
                id,
                drop: form,
                tag_options: tag_options(all_tags),
            }
            .into_response())
        }
    }
}

#[derive(Deserialize)]
pub struct MoveForm {
    status: DropStatus,
}

pub async fn r#move(
    Move { id }: Move,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(form): Form<MoveForm>,
) -> Result<Redirect, impl IntoResponse> {
    let now = chrono::Utc::now();

    let drop = match firehose::find_drop(&mut db, &session.user, id).await {
        Ok(drop) => drop,
        Err(err) => return Err(context.error(Some(session), err.into())),
    };

    let drop = firehose::move_drop(&mut db, drop, form.status, now).await;
    match drop {
        // TODO: redirect back to wherever you did this from
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.drop.id }.to_string())),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

fn bookmarklet(base_url: url::Url) -> String {
    // let href = crate::controllers::drops::New.to_string();
    let href = base_url.join(&New.to_string()).unwrap();

    format!(
        r#"javascript:(function(){{location.href="{}?title="+encodeURIComponent(document.title)+"&url="+encodeURIComponent(document.URL);}})();"#,
        href
    )
}

fn coerce_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarklet_href() -> anyhow::Result<()> {
        let expected = r#"javascript:(function(){location.href="https://example.net/firehose/drops/new?title="+encodeURIComponent(document.title)+"&url="+encodeURIComponent(document.URL);})();"#;
        assert_eq!(
            expected,
            bookmarklet(url::Url::parse("https://example.net")?),
        );
        Ok(())
    }
}
