use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::{extract::PrivateCookieJar, routing::TypedPath};
use serde::Deserialize;

use crate::{auth, models};
use crate::{AppError, Context, PgConn, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/auth/login")]
pub struct Login;

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginPage {
    context: Context,
    user: Option<models::User>,
}

pub async fn login(_: Login, context: Context, session: Option<Session>) -> impl IntoResponse {
    // No need to show the login page if they're already logged in!
    match session.map(|s| s.user) {
        Some(_user) => Redirect::to("/").into_response(),
        None => LoginPage {
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

#[derive(Template)]
#[template(path = "auth/login_confirmation.html")]
struct LoginConfirmation {
    context: Context,
    user: Option<models::User>,

    email: String,
}

pub async fn login_form(
    _: Login,
    context: Context,
    session: Option<Session>,
    State(auth): State<auth::Auth>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(session, AppError::CsrfMismatch));
    }

    let res = match auth
        .send_magic_link(form.email.clone(), Authenticate.to_string())
        .await
    {
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

#[derive(TypedPath, Deserialize)]
#[typed_path("/auth/authenticate")]
pub struct Authenticate;

#[derive(Deserialize)]
pub struct AuthenticateQuery {
    token: String,
    redirect_path: Option<String>,
}

type AuthenticateResponse = (PrivateCookieJar, Redirect);

pub async fn authenticate(
    _: Authenticate,
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    PgConn(mut db): PgConn,
    State(auth): State<auth::Auth>,
    Query(query): Query<AuthenticateQuery>,
) -> Result<AuthenticateResponse, Response> {
    let res = match auth.authenticate_magic_link(query.token).await {
        Ok(res) => res,
        Err(err) => return Err(context.error(session, err.into())),
    };
    tracing::info!("Successfully authenticated token for user {}", res.user_id);

    match auth::find_user_stytch(&mut db, res.user_id.clone()).await {
        Ok(_) => {
            let cookie = auth::session_cookie(res.session_token);

            let redirect = match query.redirect_path {
                Some(path) => Redirect::to(&path),
                None => Redirect::to("/"),
            };
            Ok((cookies.add(cookie), redirect))
        }
        Err(err) => {
            tracing::error!({ stytch_user_id = ?res.user_id, ?err }, "find user by Stytch ID");
            Err((StatusCode::BAD_REQUEST, Redirect::to(&Login.to_string())).into_response())
        }
    }
}

pub async fn authenticate_head(_: Authenticate, cookies: PrivateCookieJar) -> AuthenticateResponse {
    let cookie = auth::session_cookie("".to_string());
    let redirect = Redirect::to("/");
    (cookies.add(cookie), redirect)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/auth/logout")]
pub struct Logout;

#[derive(Deserialize, Debug)]
pub struct LogoutForm {
    authenticity_token: String,
}

pub async fn logout(
    _: Logout,
    context: Context,
    session: Option<Session>,
    cookies: PrivateCookieJar,
    State(auth): State<auth::Auth>,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(session, AppError::CsrfMismatch));
    }

    match auth::revoke_session(&auth, cookies).await {
        Ok(cookies) => {
            let session_id = session.map(|s| s.stytch.session_id);
            tracing::info!({ ?session_id }, "successfully revoked session");
            Ok((cookies, Redirect::to("/")))
        }
        Err(err) => {
            let session_id = session.as_ref().map(|s| s.stytch.session_id.clone());
            tracing::error!({ ?session_id, ?err }, "could not revoke session");
            // Fail the logout request, which may be surprising.
            //
            // By clearing the cookie, the user's browser won't know the session token anymore. But
            // anyone who _had_ somehow obtained that token would be able to use it until the
            // session naturally expired. Clicking "Log out" again shouldn't be that much of an
            // issue in the rare (🤞) case that revocation fails.
            Err(context.error(session, err.into()))
        }
    }
}
