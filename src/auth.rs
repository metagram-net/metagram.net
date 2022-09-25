use async_trait::async_trait;
use axum::{
    extract::Extension,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use sqlx::PgExecutor;
use std::sync::Arc;
use uuid::Uuid;

use crate::{models, PgConn};

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
    conn: impl PgExecutor<'_>,
    stytch_user_id: String,
) -> sqlx::Result<models::User> {
    sqlx::query_as!(
        models::User,
        r#"
        insert into users (stytch_user_id)
        values ($1)
        returning *
        "#,
        stytch_user_id,
    )
    .fetch_one(conn)
    .await
}

pub async fn find_user_stytch(
    conn: impl PgExecutor<'_>,
    stytch_user_id: String,
) -> sqlx::Result<models::User> {
    sqlx::query_as!(
        models::User,
        r#"
        select * from users
        where stytch_user_id = $1
        "#,
        stytch_user_id,
    )
    .fetch_one(conn)
    .await
}

pub async fn find_user(conn: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<models::User> {
    sqlx::query_as!(
        models::User,
        r#"
        select * from users
        where id = $1
        "#,
        user_id,
    )
    .fetch_one(conn)
    .await
}

async fn find_session(
    conn: impl PgExecutor<'_>,
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

    let user = find_user_stytch(conn, session.user_id.clone()).await?;

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

#[cfg(test)]
mod tests {
    use sqlx::{Connection, PgConnection};

    use super::*;

    // TODO: Make this a test transaction and roll it back on pass.
    async fn test_conn() -> sqlx::Result<PgConnection> {
        let url = std::env::var("TEST_DATABASE_URL").unwrap();

        PgConnection::connect(&url).await
    }

    #[tokio::test]
    async fn user_round_trip() {
        let mut conn = test_conn().await.unwrap();

        let stytch_user_id: String = uuid::Uuid::new_v4().to_string();

        let user = create_user(&mut conn, stytch_user_id.clone())
            .await
            .unwrap();
        let found = find_user_stytch(&mut conn, stytch_user_id).await.unwrap();
        assert_eq!(user, found);
    }
}
