use askama::Template;
use async_trait::async_trait;
use axum::{
    extract::{Extension, Form, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use diesel::prelude::*;
use diesel_async::AsyncPgConnection;
use diesel_async::RunQueryDsl;
use serde::Deserialize;
use std::sync::Arc;

use crate::{models, schema, AppError, Context, PgConn};

const SESSION_COOKIE_NAME: &str = "firehose_session";

pub type Auth = Arc<dyn AuthN + Send + Sync>;

#[mockall::automock]
#[async_trait]
pub trait AuthN {
    async fn send_magic_link(
        &self,
        email: String,
    ) -> stytch::Result<stytch::magic_links::email::SendResponse>;

    async fn authenticate_magic_link(
        &self,
        token: String,
    ) -> stytch::Result<stytch::magic_links::AuthenticateResponse>;

    async fn authenticate_session(
        &self,
        token: String,
    ) -> stytch::Result<stytch::sessions::AuthenticateResponse>;

    async fn revoke_session(
        &self,
        token: String,
    ) -> stytch::Result<stytch::sessions::RevokeResponse>;
}

pub fn router() -> Router {
    Router::new()
        .route("/login", get(login).post(login_form))
        .route("/logout", post(logout))
        .route("/authenticate", get(authenticate))
}

#[derive(Debug, Clone)]
pub struct Session {
    pub user: models::User,
    pub stytch: stytch::Session,
}

async fn find_session(
    db: &mut AsyncPgConnection,
    auth: &Auth,
    cookies: PrivateCookieJar<cookie::Key>,
) -> anyhow::Result<Session> {
    let session_token = cookies
        .get(SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string());

    let session = match session_token {
        None => return Err(anyhow::anyhow!("no session token in cookie")),
        Some(session_token) => {
            let res = auth.authenticate_session(session_token).await?;
            res.session
        }
    };

    use schema::users::dsl::*;

    let user: models::User = users
        .filter(stytch_user_id.eq(session.user_id.clone()))
        .get_result(db)
        .await?;

    Ok(Session {
        user,
        stytch: session,
    })
}

#[axum::async_trait]
impl<B> axum::extract::FromRequest<B> for Session
where
    B: Send,
{
    type Rejection = Response;

    async fn from_request(
        req: &mut axum::extract::RequestParts<B>,
    ) -> Result<Self, Self::Rejection> {
        let auth: Extension<Auth> = Extension::from_request(req).await.expect("extension: Auth");
        let cookies = PrivateCookieJar::from_request(req)
            .await
            .expect("PrivateCookieJar");

        let PgConn(mut db) = PgConn::from_request(req).await?;

        match find_session(&mut db, &*auth, cookies).await {
            Ok(session) => Ok(session),
            Err(err) => {
                tracing::error!({ ?err }, "no active session");
                Err(Redirect::to("/login").into_response())
            }
        }
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct Login {
    context: Context,
    user: Option<models::User>,
}

async fn login(context: Context, session: Option<Session>) -> impl IntoResponse {
    // No need to show the login page if they're already logged in!
    match session.map(|s| s.user) {
        Some(_user) => Redirect::to("/").into_response(),
        None => Login {
            context,
            user: None,
        }
        .into_response(),
    }
}

#[derive(Deserialize, Debug)]
struct LoginForm {
    authenticity_token: String,
    email: String,
}

async fn login_form(
    context: Context,
    session: Option<Session>,
    Extension(auth): Extension<Auth>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(session, AppError::CsrfMismatch));
    }

    let res = match auth.send_magic_link(form.email.clone()).await {
        Ok(res) => res,
        Err(err) => return Err(context.error(session, err.into())),
    };

    tracing::info!("Sent login link to user {}", res.user_id);
    Ok(LoginConfirmation {
        context,
        user: session.map(|s| s.user),
        email: form.email,
    })
}

#[derive(Template)]
#[template(path = "login_confirmation.html")]
struct LoginConfirmation {
    context: Context,
    user: Option<models::User>,

    email: String,
}

#[derive(Deserialize)]
struct AuthenticateQuery {
    token: String,
    redirect_path: Option<String>,
}

async fn authenticate(
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    PgConn(mut db): PgConn,
    Extension(auth): Extension<Auth>,
    Query(query): Query<AuthenticateQuery>,
) -> impl IntoResponse {
    let res = match auth.authenticate_magic_link(query.token).await {
        Ok(res) => res,
        Err(err) => return Err(context.error(session, err.into())),
    };
    tracing::info!("Successfully authenticated token for user {}", res.user_id);

    use schema::users::dsl::*;

    let user: QueryResult<models::User> = users
        .filter(stytch_user_id.eq(res.user_id.clone()))
        .get_result(&mut db)
        .await;

    match user {
        Ok(_) => {
            let cookie = Cookie::build(SESSION_COOKIE_NAME, res.session_token)
                .permanent()
                .secure(true)
                .finish();

            let redirect = match query.redirect_path {
                Some(path) => Redirect::to(&path),
                None => Redirect::to("/"),
            };
            Ok((cookies.add(cookie), redirect).into_response())
        }
        Err(err) => {
            tracing::error!({ stytch_user_id = ?res.user_id, ?err }, "find user by Stytch ID");
            Ok((StatusCode::BAD_REQUEST, Redirect::to("/login")).into_response())
        }
    }
}

#[derive(Deserialize, Debug)]
struct LogoutForm {
    authenticity_token: String,
}

async fn logout(
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    Extension(auth): Extension<Auth>,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(session, AppError::CsrfMismatch));
    }

    let session_token = cookies
        .get(SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string());
    let cookies = cookies.remove(Cookie::new(SESSION_COOKIE_NAME, ""));

    if let Some(session_token) = session_token {
        match auth.revoke_session(session_token).await {
            Ok(_res) => {
                let session_id = session.map(|s| s.stytch.session_id);
                tracing::info!({ ?session_id }, "successfully revoked session");
            }
            Err(err) => {
                let session_id = session.as_ref().map(|s| s.stytch.session_id.clone());
                tracing::error!({ ?session_id, ?err }, "could not revoke session");
                // Fail the logout request, which may be surprising.
                //
                // By clearing the cookie, the user's browser won't know the session token anymore.
                // But anyone who _had_ somehow obtained that token would be able to use it until
                // the session naturally expired. Clicking "Log out" again shouldn't be that much
                // of an issue in the rare (🤞) case that revocation fails.
                return Err(context.error(session, err.into()));
            }
        }
    }

    Ok((cookies, Redirect::to("/")))
}
