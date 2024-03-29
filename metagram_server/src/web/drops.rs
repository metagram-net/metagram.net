use std::collections::HashSet;

use askama::Template;
use axum::{
    extract::{Query, State},
    headers::{Header, Referer},
    response::{IntoResponse, Redirect},
    Router, TypedHeader,
};
use axum_extra::{
    extract::Form,
    routing::{RouterExt, TypedPath},
};
use http::HeaderValue;
use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;
use sqlx::Acquire;
use uuid::Uuid;

use crate::firehose;
use crate::models::{DropStatus, Tag, User};
use crate::{
    filters,
    view_models::{tag_options, TagOption},
};
use crate::{AppState, BaseUrl, Context, PgConn, Session};

pub fn router() -> Router<AppState> {
    Router::new()
        .typed_get(index)
        .typed_get(new)
        .typed_post(create)
        .typed_get(show)
        .typed_get(edit)
        .typed_post(update)
        .typed_post(r#move)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops")]
pub struct Collection;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/new")]
pub struct New;

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/:id")]
pub struct Member {
    id: String,
}

// TODO: Is there a good way to derive path()?

impl Member {
    pub fn path(id: &Uuid) -> String {
        Self { id: id.to_string() }.to_string()
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
    Redirect::to(&super::streams::Member::path("unread"))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct DropForm {
    title: String,
    url: String,
    tags: HashSet<String>,

    authenticity_token: String,
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

impl From<firehose::Drop> for DropForm {
    fn from(drop: firehose::Drop) -> Self {
        DropForm {
            title: drop.drop.title.unwrap_or_default(),
            url: drop.drop.url,
            tags: drop.tags.iter().map(|t| t.id.to_string()).collect(),

            ..Default::default()
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

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct ShareQuery {
    text: String,
    title: String,
    url: String,
}

impl ShareQuery {
    fn form(&self) -> DropForm {
        let mut text = self.text.trim();
        let mut title = self.title.trim();
        let mut url = self.url.trim();

        if url.is_empty() {
            (text, url) = find_url(text);
        }

        if title.is_empty() {
            title = text;
        }

        DropForm {
            title: title.to_string(),
            url: url.to_string(),
            ..Default::default()
        }
    }
}

fn find_url(text: &str) -> (&str, &str) {
    // This regex is adapted from https://bugs.chromium.org/p/chromium/issues/detail?id=789379
    lazy_static! {
        static ref RE_LAST_URL: Regex = Regex::new(r"^(.*?)\s*(https?://[^\s]+)$").unwrap();
    }

    if let Some(caps) = RE_LAST_URL.captures(text) {
        (caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str())
    } else {
        ("", "")
    }
}

pub async fn new(
    _: New,
    State(base_url): State<BaseUrl>,
    PgConn(mut db): PgConn,
    context: Context,
    session: Session,
    Query(query): Query<ShareQuery>,
) -> super::Result<impl IntoResponse> {
    let tags = firehose::list_tags(&mut db, &session.user).await?;

    let drop = query.form();

    Ok(NewDrop {
        context,
        user: Some(session.user),
        drop,
        bookmarklet: bookmarklet(base_url.0),
        tag_options: tag_options(tags),
    })
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    State(base_url): State<BaseUrl>,
    PgConn(mut db): PgConn,
    Form(mut form): Form<DropForm>,
) -> super::Result<impl IntoResponse> {
    let now = chrono::Utc::now();

    context.verify_csrf(&form.authenticity_token)?;
    form.errors = form.validate().err();

    // If the title is an empty string, set it to null instead.
    let title = Some(form.title.clone()).filter(present);

    let conn = db.acquire().await?;

    let drop = firehose::create_drop(
        conn,
        &session.user,
        title,
        form.url.clone(),
        None,
        Some(tag_selectors(&form.tags)),
        now,
    )
    .await;

    match drop {
        Ok(drop) => Ok(Redirect::to(&Member::path(&drop.drop.id)).into_response()),
        Err(err) => {
            tracing::error!({ ?err }, "could not create drop");

            let tags = firehose::list_tags(&mut db, &session.user).await?;

            Ok(NewDrop {
                context,
                user: Some(session.user),
                drop: form,
                bookmarklet: bookmarklet(base_url.0),
                tag_options: tag_options(tags),
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
) -> super::Result<impl IntoResponse> {
    let id = parse_drop_id(&id)?;
    let drop = firehose::find_drop(&mut db, &session.user, id).await?;

    Ok(Show {
        context,
        user: Some(session.user),
        drop,
    })
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
) -> super::Result<impl IntoResponse> {
    let drop = firehose::find_drop(&mut db, &session.user, id).await?;
    let tags = firehose::list_tags(&mut db, &session.user).await?;

    Ok(EditDrop {
        context,
        user: Some(session.user),
        id,
        drop: drop.into(),
        tag_options: tag_options(tags),
    })
}

pub async fn update(
    Member { id }: Member,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
    Form(mut form): Form<DropForm>,
) -> super::Result<impl IntoResponse> {
    context.verify_csrf(&form.authenticity_token)?;
    form.errors = form.validate().err();

    let id = parse_drop_id(&id)?;
    let drop = firehose::find_drop(&mut db, &session.user, id).await?;

    let fields = firehose::DropFields {
        title: Some(form.title.clone()).filter(present),
        url: Some(form.url.clone()).filter(present),
    };
    let tags = tag_selectors(&form.tags);

    let drop = firehose::update_drop(&mut db, &session.user, &drop.drop, fields, Some(tags)).await;
    match drop {
        Ok(drop) => Ok(Redirect::to(&Member::path(&drop.drop.id)).into_response()),
        Err(err) => {
            tracing::error!({ ?err }, "could not update drop");

            let tags = firehose::list_tags(&mut db, &session.user).await?;

            Ok(EditDrop {
                context,
                user: Some(session.user),
                id,
                drop: form,
                tag_options: tag_options(tags),
            }
            .into_response())
        }
    }
}

#[derive(Deserialize)]
pub struct MoveForm {
    status: DropStatus,

    authenticity_token: String,
}

pub async fn r#move(
    Move { id }: Move,
    Back { return_path }: Back,
    session: Session,
    PgConn(mut db): PgConn,
    context: Context,
    Form(form): Form<MoveForm>,
) -> super::Result<impl IntoResponse> {
    let now = chrono::Utc::now();

    context.verify_csrf(&form.authenticity_token)?;

    let drop = firehose::find_drop(&mut db, &session.user, id).await?;
    let drop = firehose::move_drop(&mut db, drop, form.status, now).await?;

    // Redirect back to the page the action was taken from. If we don't know, go to the
    // drop page.
    let dest = return_path.unwrap_or_else(|| Member::path(&drop.drop.id));
    Ok(Redirect::to(&dest))
}

fn bookmarklet(base_url: url::Url) -> String {
    let href = base_url.join(&New.to_string()).unwrap();

    format!(
        r#"javascript:(function(){{location.href="{}?title="+encodeURIComponent(document.title)+"&url="+encodeURIComponent(document.URL);}})();"#,
        href
    )
}

pub struct Back {
    return_path: Option<String>,
}

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for Back
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let value = TypedHeader::<Referer>::from_request_parts(parts, state)
            .await
            .ok();

        let return_path = match value {
            None => None,
            Some(value) => {
                let mut paths = Vec::<HeaderValue>::new();
                value.encode(&mut paths);
                paths
                    .get(0)
                    .and_then(|p| p.to_str().ok())
                    .map(|s| s.to_string())
                    .filter(present)
            }
        };

        Ok(Self { return_path })
    }
}

fn present(s: &String) -> bool {
    !s.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarklet_href() {
        let expected = r#"javascript:(function(){location.href="https://example.net/firehose/drops/new?title="+encodeURIComponent(document.title)+"&url="+encodeURIComponent(document.URL);})();"#;
        assert_eq!(
            expected,
            bookmarklet(url::Url::parse("https://example.net").unwrap()),
        );
    }

    #[test]
    fn share_query_form() {
        let form = ShareQuery::default().form();
        assert_eq!("", form.title);
        assert_eq!("", form.url);

        let form = ShareQuery {
            text: "https://example.com/sample".to_string(),
            ..Default::default()
        }
        .form();
        assert_eq!("", form.title);
        assert_eq!("https://example.com/sample", form.url);

        let form = ShareQuery {
            text: "Shared from Twitter: https://example.com/sample".to_string(),
            ..Default::default()
        }
        .form();
        assert_eq!("Shared from Twitter:", form.title);
        assert_eq!("https://example.com/sample", form.url);

        let form = ShareQuery {
            text: r#"Watch "this video" on ..."#.to_string(),
            url: "https://example.com/sample".to_string(),
            ..Default::default()
        }
        .form();
        assert_eq!("Watch \"this video\" on ...", form.title);
        assert_eq!("https://example.com/sample", form.url);

        let form = ShareQuery {
            title: "Test Title".to_string(),
            url: "https://example.com/sample".to_string(),
            ..Default::default()
        }
        .form();
        assert_eq!("Test Title", form.title);
        assert_eq!("https://example.com/sample", form.url);
    }
}

fn parse_drop_id(id: &str) -> super::Result<Uuid> {
    Uuid::parse_str(id).map_err(|_| super::Error::DropNotFound {
        drop_id: id.to_string(),
    })
}
