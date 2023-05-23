use askama::Template;
use axum::{
    middleware,
    response::{IntoResponse, Redirect, Response},
    Router,
};
use http::StatusCode;

use crate::{auth::Session, AppState, Context, User};

pub mod auth;
pub mod drops;
pub mod firehose;
pub mod home;
pub mod hydrants;
pub mod streams;
pub mod tags;
pub mod whoops;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(drops::router())
        .merge(firehose::router())
        .merge(home::router())
        .merge(hydrants::router())
        .merge(streams::router())
        .merge(tags::router())
        .merge(whoops::router())
        .route_layer(middleware::map_response_with_state(state, show_app_error))
}

async fn show_app_error(
    ctx: Context,
    session: Option<Session>,
    mut res: Response,
) -> impl IntoResponse {
    let web_error = res.extensions_mut().remove::<Error>();

    if let Some(err) = web_error {
        tracing::error!("{:?}", err);

        return err.render(ctx, session);
    }

    res
}

pub type Result<T> = std::result::Result<T, Error>;

#[allow(clippy::large_enum_variant)] // TODO: Box more
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("authenticity token mismatch")]
    CsrfMismatch { cookie: String, form: String },

    #[error("not logged in")]
    NotLoggedIn,

    #[error("user not found")]
    UserNotFound { stytch_user_id: String },

    #[error("drop not found")]
    DropNotFound { drop_id: String },

    #[error("stream not found")]
    StreamNotFound { stream_id: String },

    #[error(transparent)]
    Stytch(#[from] stytch::Error),

    #[error(transparent)]
    Boxed(#[from] axum::BoxError),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl Error {
    pub fn boxed(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Error::Boxed(err.into())
    }
}

// Create a fallback response to show in the rare case that something goes wrong in the
// error mapper.
//
// Put the real error (self) in the response extensions.
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut res = StatusCode::INTERNAL_SERVER_ERROR.into_response();

        res.extensions_mut().insert(self);

        res
    }
}

impl Error {
    pub fn render(&self, context: Context, session: Option<Session>) -> Response {
        let user = session.map(|s| s.user);

        fn wrap(res: impl IntoResponse) -> Response {
            res.into_response()
        }

        use Error::*;
        match self {
            CsrfMismatch { .. } => wrap((
                StatusCode::UNPROCESSABLE_ENTITY,
                UnprocessableEntity { context, user },
            )),

            NotLoggedIn => wrap(Redirect::to(&auth::Login.to_string())),

            UserNotFound { .. } => wrap(Redirect::to(&auth::Login.to_string())),

            // TODO: Render a nicer not-found
            DropNotFound { .. } => wrap(StatusCode::NOT_FOUND),

            // TODO: Render a nicer not-found
            StreamNotFound { .. } => wrap(StatusCode::NOT_FOUND),

            Stytch(_) | Boxed(_) | Anyhow(_) | Sqlx(_) => wrap((
                StatusCode::INTERNAL_SERVER_ERROR,
                InternalServerError { context, user },
            )),
        }
    }
}

#[derive(Template)]
#[template(path = "errors/422_unprocessable_entity.html")]
struct UnprocessableEntity {
    context: Context,
    user: Option<User>,
}

#[derive(Template)]
#[template(path = "errors/500_internal_server_error.html")]
struct InternalServerError {
    context: Context,
    user: Option<User>,
}

// TODO: Web prelude = {Result, Error, StdResult}
