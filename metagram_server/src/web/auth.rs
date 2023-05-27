use askama::Template;
use axum::{
    extract::{Form, Query, State},
    response::{IntoResponse, Redirect},
    Router,
};
use axum_extra::{
    extract::PrivateCookieJar,
    routing::{RouterExt, TypedPath},
};
use serde::Deserialize;

use crate::{auth, models};
use crate::{AppState, Context, PgConn, Session};

pub fn router() -> Router<AppState> {
    Router::new()
        .typed_get(login)
        .typed_post(login_form)
        .typed_post(logout)
        .typed_get(authenticate)
        .typed_head(authenticate_head)
}

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
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> super::Result<impl IntoResponse> {
    context.verify_csrf(&form.authenticity_token)?;

    let res = state
        .auth
        .send_magic_link(form.email.clone(), Authenticate.to_string())
        .await?;

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
    cookies: PrivateCookieJar,
    PgConn(mut db): PgConn,
    State(auth): State<auth::Auth>,
    Query(query): Query<AuthenticateQuery>,
) -> super::Result<AuthenticateResponse> {
    let res = auth.authenticate_magic_link(query.token).await?;
    let stytch_user_id = res.user_id.clone();

    tracing::info!({ ?stytch_user_id }, "authenticated Stytch session");

    match auth::find_user_stytch(&mut db, stytch_user_id.clone()).await {
        Ok(_) => {
            let cookie = auth::session_cookie(res.session_token);

            let redirect = match query.redirect_path {
                Some(path) => Redirect::to(&path),
                None => Redirect::to("/"),
            };
            Ok((cookies.add(cookie), redirect))
        }
        Err(err) => {
            tracing::error!({ ?stytch_user_id, ?err }, "find user by Stytch ID");
            Err(super::Error::UserNotFound { stytch_user_id })
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
) -> super::Result<impl IntoResponse> {
    context.verify_csrf(&form.authenticity_token)?;

    match auth::revoke_session(&auth, cookies).await {
        Ok(cookies) => {
            let session_id = session.map(|s| s.stytch.session_id);
            tracing::info!({ ?session_id }, "revoked session");

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
            Err(err.into())
        }
    }
}
