use std::collections::HashSet;

use askama::Template;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::{extract::Form, routing::TypedPath};
use http::StatusCode;
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use crate::firehose;
use crate::models::User;
use crate::{
    filters,
    view_models::{tag_options, TagOption},
};
use crate::{Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants")]
pub struct Collection;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants/new")]
pub struct New;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants/:id")]
pub struct Member {
    id: Uuid,
}

impl Member {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants/:id/edit")]
pub struct Edit {
    id: Uuid,
}

impl Edit {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants/:id/move")]
pub struct Move {
    id: Uuid,
}

#[derive(Template)]
#[template(path = "firehose/hydrants/index.html")]
struct Index {
    context: Context,
    user: Option<User>,
    hydrants: Vec<firehose::Hydrant>,
}

pub async fn index(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let hydrants = firehose::list_hydrants(&mut db, &session.user).await;

    match hydrants {
        Ok(hydrants) => Ok(Index {
            context,
            user: Some(session.user),
            hydrants,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

#[derive(Default, Deserialize)]
pub struct HydrantForm {
    name: String,
    url: String,
    #[serde(default, deserialize_with = "checkbox")]
    active: bool,
    tags: HashSet<String>,

    errors: Option<Vec<String>>,
}

fn checkbox<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_ref() {
        "on" => Ok(true),
        _ => Ok(false),
    }
}

// TODO: I bet this can be derived
impl HydrantForm {
    fn validate(&self) -> Result<(), Vec<String>> {
        if let Some(errors) = &self.errors {
            return Err(errors.to_vec());
        }

        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("Name cannot be blank".to_string());
        }
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
#[template(path = "firehose/hydrants/new.html")]
struct NewHydrant {
    context: Context,
    user: Option<User>,
    hydrant: HydrantForm,
    tag_options: Vec<TagOption>,
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

    Ok(NewHydrant {
        context,
        user: Some(session.user),
        hydrant: HydrantForm {
            active: true,
            ..Default::default()
        },
        tag_options: tag_options(tags),
    })
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(mut form): Form<HydrantForm>,
) -> Result<Redirect, Response> {
    let errors = match form.validate() {
        Ok(_) => None,
        Err(errors) => Some(errors),
    };
    form.errors = errors;

    let hydrant = firehose::create_hydrant(
        &mut db,
        &session.user,
        &form.name,
        &form.url,
        form.active,
        Some(tag_selectors(&form.tags)),
    )
    .await;
    match hydrant {
        Ok(hydrant) => Ok(Redirect::to(
            &Member {
                id: hydrant.hydrant.id,
            }
            .to_string(),
        )),
        Err(err) => {
            tracing::error!({ ?err }, "could not create hydrant");

            let tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(NewHydrant {
                context,
                user: Some(session.user),
                hydrant: form,
                tag_options: tag_options(tags),
            }
            .into_response())
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/hydrants/show.html")]
struct Show {
    context: Context,
    user: Option<User>,
    hydrant: firehose::Hydrant,
}

pub async fn show(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let hydrant = match firehose::find_hydrant(&mut db, &session.user, id).await {
        Ok(hydrant) => hydrant,
        Err(err) => return Err(context.error(Some(session), err.into())),
    };

    // TODO: show hydrant drops?

    Ok(Show {
        context,
        user: Some(session.user),
        hydrant,
    })
}

#[derive(Template)]
#[template(path = "firehose/hydrants/edit.html")]
struct EditHydrant {
    context: Context,
    user: Option<User>,
    id: Uuid,
    hydrant: HydrantForm,
    tag_options: Vec<TagOption>,
}

pub async fn edit(
    Edit { id }: Edit,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let hydrant = match firehose::find_hydrant(&mut db, &session.user, id).await {
        Ok(hydrant) => hydrant,
        Err(_) => return Err(StatusCode::NOT_FOUND.into_response()),
    };

    let selected_tags: HashSet<String> = hydrant.tags.iter().map(|t| t.id.to_string()).collect();

    let all_tags = match firehose::list_tags(&mut db, &session.user).await {
        Ok(tags) => tags,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    Ok(EditHydrant {
        context,
        user: Some(session.user),
        id,
        hydrant: HydrantForm {
            errors: None,
            name: hydrant.hydrant.name,
            url: hydrant.hydrant.url,
            active: hydrant.hydrant.active,
            tags: selected_tags,
        },
        tag_options: tag_options(all_tags),
    })
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(form): Form<HydrantForm>,
) -> Result<Redirect, Response> {
    let hydrant = match firehose::find_hydrant(&mut db, &session.user, id).await {
        Ok(hydrant) => hydrant,
        Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
    };

    let tags = tag_selectors(&form.tags);

    let fields = firehose::HydrantFields {
        name: Some(form.name.clone()),
        url: Some(form.url.clone()),
        active: Some(form.active),
        tags: Some(tags),
    };

    match firehose::update_hydrant(&mut db, &session.user, &hydrant.hydrant, fields).await {
        Ok(hydrant) => Ok(Redirect::to(
            &Member {
                id: hydrant.hydrant.id,
            }
            .to_string(),
        )),
        Err(err) => {
            tracing::error!({ ?err }, "could not update hydrant");

            let tags = match firehose::list_tags(&mut db, &session.user).await {
                Ok(tags) => tags,
                Err(err) => return Err(context.error(Some(session), err.into()).into_response()),
            };

            Err(EditHydrant {
                context,
                user: Some(session.user),
                id,
                hydrant: form,
                tag_options: tag_options(tags),
            }
            .into_response())
        }
    }
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/hydrants/:id/delete")]
pub struct Delete {
    id: Uuid,
}

impl Delete {
    pub fn path(id: &Uuid) -> String {
        Self { id: *id }.to_string()
    }
}

pub async fn delete(
    Delete { id }: Delete,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<Redirect, Response> {
    let hydrant = match firehose::find_hydrant(&mut db, &session.user, id).await {
        Ok(hydrant) => hydrant,
        Err(_) => return Err(StatusCode::NOT_FOUND.into_response()),
    };

    match firehose::delete_hydrant(&mut db, &session.user, hydrant.hydrant).await {
        Ok(_) => Ok(Redirect::to(&Collection.to_string())),
        Err(err) => Err(context.error(Some(session), err.into()).into_response()),
    }
}

// TODO: Third copy, extract it.
fn tag_selectors(opts: &HashSet<String>) -> Vec<firehose::TagSelector> {
    opts.iter()
        // Keep this prefix synced with the select2 options.
        .filter_map(|value| match value.strip_prefix('_') {
            Some(name) => Some(firehose::TagSelector::Create {
                name: name.to_string(),
                color: firehose::Tag::DEFAULT_COLOR.to_string(),
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
