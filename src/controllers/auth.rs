use askama::Template;
use axum::{
    extract::{Extension, Form, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Deserialize;

use crate::{auth, models, schema};
use crate::{AppError, Context, PgConn, Session};

#[derive(Template)]
#[template(path = "login.html")]
struct Login {
    context: Context,
    user: Option<models::User>,
}

pub async fn login(context: Context, session: Option<Session>) -> impl IntoResponse {
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
pub struct LoginForm {
    authenticity_token: String,
    email: String,
}

pub async fn login_form(
    context: Context,
    session: Option<Session>,
    Extension(auth): Extension<auth::Auth>,
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
pub struct AuthenticateQuery {
    token: String,
    redirect_path: Option<String>,
}

pub async fn authenticate(
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    PgConn(mut db): PgConn,
    Extension(auth): Extension<auth::Auth>,
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
            let cookie = Cookie::build(auth::SESSION_COOKIE_NAME, res.session_token)
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
pub struct LogoutForm {
    authenticity_token: String,
}

pub async fn logout(
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    Extension(auth): Extension<auth::Auth>,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(session, AppError::CsrfMismatch));
    }

    let session_token = cookies
        .get(auth::SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string());
    let cookies = cookies.remove(Cookie::new(auth::SESSION_COOKIE_NAME, ""));

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
