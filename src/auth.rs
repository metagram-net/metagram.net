use async_trait::async_trait;
use axum::{
    extract::Extension,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use diesel::prelude::*;
use diesel_async::AsyncPgConnection;
use diesel_async::RunQueryDsl;
use std::sync::Arc;

use crate::{models, schema, PgConn};

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

#[derive(Debug, Clone)]
pub struct Session {
    pub user: models::User,
    pub stytch: stytch::Session,
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

pub async fn create_user(
    db: &mut AsyncPgConnection,
    stytch_user_id: String,
) -> anyhow::Result<models::User> {
    use diesel::insert_into;
    use schema::users::dsl as t;

    let user: models::User = insert_into(t::users)
        .values(&models::NewUser {
            stytch_user_id: &stytch_user_id,
        })
        .get_result(db)
        .await?;
    Ok(user)
}

pub async fn find_user(
    db: &mut AsyncPgConnection,
    stytch_user_id: String,
) -> anyhow::Result<models::User> {
    use schema::users::dsl as t;

    let user: models::User = t::users
        .filter(t::stytch_user_id.eq(stytch_user_id))
        .get_result(db)
        .await?;
    Ok(user)
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

pub async fn revoke_session(
    auth: &Auth,
    cookies: PrivateCookieJar<cookie::Key>,
) -> anyhow::Result<PrivateCookieJar<cookie::Key>> {
    let session_token = cookies
        .get(SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string());

    let session_token = match session_token {
        // Nothing to do!
        None => return Ok(cookies),
        Some(token) => token,
    };

    let _res = auth.revoke_session(session_token).await?;
    Ok(cookies.remove(Cookie::new(SESSION_COOKIE_NAME, "")))
}

pub fn session_cookie(session_token: String) -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME, session_token)
        .permanent()
        .secure(true)
        .finish()
}
