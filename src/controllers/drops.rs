use askama::Template;
use axum::{
    extract::Form,
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::routing::TypedPath;
use serde::Deserialize;
use uuid::Uuid;

use crate::firehose;
use crate::models::{Drop, DropStatus, User};
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

#[derive(TypedPath, Deserialize)]
#[typed_path("/firehose/drops/:id/move")]
pub struct Move {
    id: Uuid,
}

pub async fn index(_: Collection) -> Redirect {
    Redirect::to("/firehose/streams/unread")
}

// TODO: Form Option<String> with empty-to-None
#[derive(Default, Deserialize)]
pub struct DropForm {
    title: String,
    url: String,
}

#[derive(Template)]
#[template(path = "firehose/drops/new.html")]
struct NewDrop {
    context: Context,
    user: Option<User>,
    drop: DropForm,
    bookmarklet: String,
}

pub async fn new(
    _: New,
    Extension(base_url): Extension<BaseUrl>,
    context: Context,
    session: Session,
) -> impl IntoResponse {
    NewDrop {
        context,
        user: Some(session.user),
        drop: Default::default(),
        bookmarklet: bookmarklet(base_url.0),
    }
}

pub async fn create(
    _: Collection,
    context: Context,
    session: Session,
    Extension(base_url): Extension<BaseUrl>,
    PgConn(mut db): PgConn,
    Form(form): Form<DropForm>,
) -> Result<Redirect, impl IntoResponse> {
    let now = chrono::Utc::now();
    let user = session.user;
    let title = if form.title.is_empty() {
        None
    } else {
        Some(form.title.clone())
    };

    // TODO: Validate the fields?
    let drop = firehose::create_drop(&mut db, &user, title, form.url.clone(), now).await;
    match drop {
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not create drop");
            Err(NewDrop {
                context,
                user: Some(user),
                drop: form,
                bookmarklet: bookmarklet(base_url.0),
            })
        }
    }
}

#[derive(Template)]
#[template(path = "firehose/drops/show.html")]
struct Show {
    context: Context,
    user: Option<User>,
    drop: Drop,
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
}

pub async fn edit(
    Edit { id }: Edit,
    context: Context,
    session: Session,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let drop = firehose::find_drop(&mut db, &session.user, id).await;
    match drop {
        Ok(drop) => Ok(EditDrop {
            context,
            user: Some(session.user),
            id,
            drop: DropForm {
                title: drop.title.unwrap_or_default(),
                url: drop.url,
            },
        }),
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

    let fields = {
        let title = if form.title.is_empty() {
            None
        } else {
            Some(form.title.clone())
        };

        let url = if form.url.is_empty() {
            None
        } else {
            Some(form.url.clone())
        };

        firehose::DropFields { title, url }
    };

    let drop = firehose::update_drop(&mut db, &drop, fields).await;
    match drop {
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.id }.to_string())),
        Err(err) => {
            tracing::error!({ ?err }, "could not update drop");
            Err(EditDrop {
                context,
                user: Some(session.user),
                id,
                drop: form,
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

    let drop = firehose::move_drop(&mut db, &drop, form.status, now).await;
    match drop {
        // TODO: redirect back to wherever you did this from
        Ok(drop) => Ok(Redirect::to(&Member { id: drop.id }.to_string())),
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
