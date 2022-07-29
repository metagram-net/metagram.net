use askama::Template;
use axum::extract::Path;
use axum::{
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::routing::Resource;
use diesel_async::AsyncPgConnection;

use crate::models::{Drop, DropStatus, NewDrop, User};
use crate::{schema, Context, PgConn, Session};

pub fn router() -> Router {
    let drops = Resource::named("drops")
        .index(drops::index)
        .new(drops::new)
        .create(drops::create)
        .show(drops::show)
        .edit(drops::edit)
        .update(drops::update)
        .nest(Router::new().route("/move", post(drops::r#move)));

    Router::new()
        .route("/", get(index))
        .route("/about", get(about))
        .route("/streams/:id", get(stream))
        .merge(drops)
}

async fn index(session: Option<Session>) -> impl IntoResponse {
    match session {
        None => Redirect::to("/firehose/about"),
        Some(_) => Redirect::to("/firehose/streams/unread"),
    }
}

#[derive(Template)]
#[template(path = "firehose/about.html")]
struct About {
    context: Context,
    user: Option<User>,
}

async fn about(context: Context, session: Option<Session>) -> impl IntoResponse {
    About {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(Template)]
#[template(path = "firehose/stream.html")]
struct ShowStream {
    context: Context,
    user: Option<User>,
    drops: Vec<Drop>,
}

async fn stream(
    context: Context,
    session: Session,
    Path(id): Path<String>,
    PgConn(mut db): PgConn,
) -> Result<impl IntoResponse, Response> {
    let drops: anyhow::Result<Vec<Drop>> = match id.as_str() {
        "unread" => list_drops(&mut db, session.user.clone(), DropStatus::Unread).await,
        "read" => list_drops(&mut db, session.user.clone(), DropStatus::Read).await,
        "saved" => list_drops(&mut db, session.user.clone(), DropStatus::Saved).await,
        _id => todo!("feat: custom streams"),
    };

    match drops {
        Ok(drops) => Ok(ShowStream {
            context,
            user: Some(session.user),
            drops,
        }),
        Err(err) => Err(context.error(Some(session), err.into())),
    }
}

async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    status: DropStatus,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::status.eq(status)))
        .load(db)
        .await?;
    Ok(res)
}

pub async fn create_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    title: Option<String>,
    url: String,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: Drop = insert_into(t::drops)
        .values(&NewDrop {
            user_id: user.id,
            title: title.as_deref(),
            url: &url,
            status: DropStatus::Unread,
            moved_at: now.naive_utc(),
        })
        .get_result(db)
        .await?;
    Ok(drop)
}

#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::drops)]
pub struct DropFields {
    pub title: Option<String>,
    pub url: Option<String>,
}

pub async fn update_drop(
    db: &mut AsyncPgConnection,
    drop: &Drop,
    fields: DropFields,
) -> anyhow::Result<Drop> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let drop: Drop = update(drop).set(fields).get_result(db).await?;
    Ok(drop)
}

pub async fn move_drop(
    db: &mut AsyncPgConnection,
    drop: &Drop,
    status: DropStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::{update, ExpressionMethods};
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: Drop = update(drop)
        .set((t::status.eq(status), t::moved_at.eq(now.naive_utc())))
        .get_result(db)
        .await?;
    Ok(drop)
}

mod drops {
    use askama::Template;
    use axum::extract::{Form, Path};
    use axum::response::{IntoResponse, Redirect, Response};
    use diesel_async::AsyncPgConnection;
    use serde::Deserialize;

    use crate::models::{Drop, DropStatus, User};
    use crate::{schema, Context, PgConn, Session};

    pub async fn index() -> Redirect {
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
    }

    pub async fn new(context: Context, session: Session) -> impl IntoResponse {
        NewDrop {
            context,
            user: Some(session.user),
            drop: Default::default(),
        }
    }

    pub async fn create(
        context: Context,
        session: Session,
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
        let drop = super::create_drop(&mut db, &user, title, form.url.clone(), now).await;
        match drop {
            Ok(drop) => Ok(Redirect::to(&format!("/firehose/drops/{}", drop.id))), // TODO: named route generation
            Err(err) => {
                tracing::error!({ ?err }, "could not create drop");
                Err(NewDrop {
                    context,
                    user: Some(user),
                    drop: form,
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
        context: Context,
        session: Session,
        Path(id): Path<uuid::Uuid>,
        PgConn(mut db): PgConn,
    ) -> Result<impl IntoResponse, Response> {
        match find_drop(&mut db, &session.user, id).await {
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
        id: String,
        drop: DropForm,
    }

    pub async fn edit(
        context: Context,
        session: Session,
        Path(id): Path<uuid::Uuid>,
        PgConn(mut db): PgConn,
    ) -> Result<impl IntoResponse, Response> {
        let drop = find_drop(&mut db, &session.user, id).await;
        match drop {
            Ok(drop) => Ok(EditDrop {
                context,
                user: Some(session.user),
                id: id.to_string(),
                drop: DropForm {
                    title: drop.title.unwrap_or_default(),
                    url: drop.url,
                },
            }),
            Err(err) => Err(context.error(Some(session), err.into())),
        }
    }

    pub async fn update(
        context: Context,
        session: Session,
        PgConn(mut db): PgConn,
        Path(id): Path<uuid::Uuid>,
        Form(form): Form<DropForm>,
    ) -> Result<Redirect, Response> {
        let drop = match find_drop(&mut db, &session.user, id).await {
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

            super::DropFields { title, url }
        };

        let drop = super::update_drop(&mut db, &drop, fields).await;
        match drop {
            Ok(drop) => Ok(Redirect::to(&format!("/firehose/drops/{}", drop.id))), // TODO: named route generation
            Err(err) => {
                tracing::error!({ ?err }, "could not update drop");
                Err(EditDrop {
                    context,
                    user: Some(session.user),
                    id: id.to_string(),
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
        context: Context,
        session: Session,
        PgConn(mut db): PgConn,
        Path(id): Path<uuid::Uuid>,
        Form(form): Form<MoveForm>,
    ) -> Result<Redirect, impl IntoResponse> {
        let now = chrono::Utc::now();

        let drop = match find_drop(&mut db, &session.user, id).await {
            Ok(drop) => drop,
            Err(err) => return Err(context.error(Some(session), err.into())),
        };

        let drop = super::move_drop(&mut db, &drop, form.status, now).await;
        match drop {
            // TODO: redirect back to wherever you did this from
            Ok(drop) => Ok(Redirect::to(&format!("/firehose/drops/{}", drop.id))), // TODO: named route generation
            Err(err) => Err(context.error(Some(session), err.into())),
        }
    }

    async fn find_drop(
        db: &mut AsyncPgConnection,
        user: &User,
        id: uuid::Uuid,
    ) -> anyhow::Result<Drop> {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        use schema::drops::dsl as t;

        let res = t::drops
            .filter(t::user_id.eq(user.id).and(t::id.eq(id)))
            .get_result(db)
            .await?;
        Ok(res)
    }
}
