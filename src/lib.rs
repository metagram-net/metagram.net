#[macro_use]
extern crate diesel;

use askama::Template;
use async_trait::async_trait;
use axum::{
    extract::{Extension, Form, Query, RequestParts},
    handler::Handler,
    http::{Request, StatusCode},
    response::{IntoResponse, IntoResponseParts, Redirect, Response, ResponseParts},
    routing::{get, post},
    Json, Router,
};
use axum_csrf::{CsrfLayer, CsrfToken};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use derivative::Derivative;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_async::{
    pooled_connection::deadpool::{self, Object, Pool},
    AsyncPgConnection,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;

pub mod models;
pub mod schema;

use models::User;

const SESSION_COOKIE_NAME: &str = "firehose_session";

type PgPool = Pool<AsyncPgConnection>;

#[derive(Debug, Clone)]
pub struct Session {
    user: User,
    stytch: stytch::Session,
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

    let user: User = users
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
        let pool: Extension<PgPool> = Extension::from_request(req)
            .await
            .expect("extension: PgPool");
        let auth: Extension<Auth> = Extension::from_request(req).await.expect("extension: Auth");
        let cookies = PrivateCookieJar::from_request(req)
            .await
            .expect("PrivateCookieJar");
        let context = Context::from_request(req).await.expect("PrivateCookieJar");

        let mut db = match pool.get().await {
            Ok(conn) => conn,
            Err(err) => return Err(context.error(None, err.into())),
        };

        match find_session(&mut db, &*auth, cookies).await {
            Ok(session) => Ok(session),
            Err(err) => {
                tracing::error!({ ?err }, "no active session");
                Err(Redirect::to("/login").into_response())
            }
        }
    }
}

struct PgConn(Object<AsyncPgConnection>);

#[axum::async_trait]
impl<B> axum::extract::FromRequest<B> for PgConn
where
    B: Send,
{
    type Rejection = Response;

    async fn from_request(
        req: &mut axum::extract::RequestParts<B>,
    ) -> Result<Self, Self::Rejection> {
        let pool: Extension<PgPool> = Extension::from_request(req)
            .await
            .expect("extension: PgPool");
        let context = Context::from_request(req).await.expect("PrivateCookieJar");

        match pool.get().await {
            Ok(conn) => Ok(PgConn(conn)),
            Err(err) => Err(context.error(None, err.into())),
        }
    }
}

pub struct ServerConfig {
    pub cookie_key: cookie::Key,
    pub database_pool: PgPool,
    pub auth: Auth,
}

pub struct Server {
    router: Router,
}

pub type Auth = Arc<dyn AuthN + Send + Sync>;

impl Server {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let router = Router::new()
            .route("/", get(index))
            .route("/login", get(login).post(login_form))
            .route("/logout", post(logout))
            .route("/authenticate", get(authenticate))
            .route("/.well-known/health-check", get(health_check))
            .route("/whoops/500", get(whoops_500))
            .route("/whoops/422", get(whoops_422))
            .fallback(not_found.into_service());

        let trace_layer = TraceLayer::new_for_http()
            .make_span_with(
                DefaultMakeSpan::new()
                    .level(Level::INFO)
                    .include_headers(true),
            )
            .on_request(DefaultOnRequest::new().level(Level::INFO))
            .on_response(
                DefaultOnResponse::new()
                    .level(Level::INFO)
                    .include_headers(true),
            );

        let app = router
            .layer(
                ServiceBuilder::new()
                    // To have request IDs show up in traces, the tracing middleware has to be
                    // _between_ the request_id ones.
                    .set_x_request_id(tower_http::request_id::MakeRequestUuid)
                    .layer(trace_layer)
                    .propagate_x_request_id(),
            )
            .layer(Extension::<PgPool>(config.database_pool))
            .layer(Extension::<cookie::Key>(config.cookie_key.clone()))
            .layer(Extension::<Auth>(config.auth))
            // This ordering is important! While processing the inbound request, auto_csrf_token
            // assumes that CsrfLayer has already extracted the authenticity token from the
            // cookies. When generating the outbound response, order doesn't matter. So keep
            // auto_csrf_token deeper in the middleware stack.
            //
            // TODO: Could this become CsrfLayer's job?
            .layer(axum::middleware::from_fn(auto_csrf_token))
            .layer(CsrfLayer::build().key(config.cookie_key).finish());

        Ok(Self { router: app })
    }

    pub async fn run(self, addr: SocketAddr) -> hyper::Result<()> {
        tracing::info!("Listening on http://{}", addr);
        axum::Server::bind(&addr)
            .serve(self.router.into_make_service())
            .await
    }
}

async fn auto_csrf_token<B: Send>(
    req: Request<B>,
    next: axum::middleware::Next<B>,
) -> impl IntoResponse {
    let mut parts = RequestParts::new(req);
    let csrf_token: CsrfToken = parts.extract().await.expect("layer: CsrfToken");

    let req = parts.try_into_request().expect("into request");
    (csrf_token, next.run(req).await)
}

#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("authenticity token mismatch")]
    CsrfMismatch,

    #[error(transparent)]
    StytchError(#[from] stytch::Error),

    #[error(transparent)]
    DeadpoolError(#[from] deadpool::PoolError),

    #[error(transparent)]
    Unhandled(#[from] anyhow::Error),
}

#[derive(Serialize)]
struct Health {
    status: String,
}

async fn health_check() -> Json<Health> {
    let health = Health {
        status: "Ok".to_string(),
    };
    Json(health)
}

#[derive(Clone, Derivative)]
#[derivative(Debug)]
struct Context {
    #[derivative(Debug = "ignore")]
    csrf_token: CsrfToken,
    request_id: Option<String>,
}

impl Context {
    fn error(self, session: Option<Session>, err: AppError) -> Response {
        tracing::error!("{:?}", err);

        let user = session.map(|s| s.user);

        use AppError::*;
        match err {
            CsrfMismatch => (
                StatusCode::UNPROCESSABLE_ENTITY,
                UnprocessableEntity {
                    context: self,
                    user,
                },
            )
                .into_response(),
            StytchError(err) => {
                tracing::error!({ ?err }, "stytch error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InternalServerError {
                        context: self,
                        user,
                    },
                )
                    .into_response()
            }
            DeadpoolError(err) => {
                tracing::error!({ ?err }, "deadpool error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InternalServerError {
                        context: self,
                        user,
                    },
                )
                    .into_response()
            }
            Unhandled(err) => {
                tracing::error!({ ?err }, "unhandled error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InternalServerError {
                        context: self,
                        user,
                    },
                )
                    .into_response()
            }
        }
    }
}

#[axum::async_trait]
impl<B> axum::extract::FromRequest<B> for Context
where
    B: Send,
{
    type Rejection = std::convert::Infallible;

    async fn from_request(
        req: &mut axum::extract::RequestParts<B>,
    ) -> Result<Self, Self::Rejection> {
        let csrf_token = CsrfToken::from_request(req)
            .await
            .expect("layer: CsrfToken");

        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Ok(Self {
            csrf_token,
            request_id,
        })
    }
}

impl IntoResponseParts for Context {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        self.csrf_token.into_response_parts(res)
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    context: Context,
    user: Option<User>,
}

async fn index(context: Context, session: Option<Session>) -> impl IntoResponse {
    Index {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct Login {
    context: Context,
    user: Option<User>,
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
    user: Option<User>,

    email: String,
}

#[derive(Template)]
#[template(path = "500_internal_server_error.html")]
struct InternalServerError {
    context: Context,
    user: Option<User>,
}

#[derive(Template)]
#[template(path = "422_unprocessable_entity.html")]
struct UnprocessableEntity {
    context: Context,
    user: Option<User>,
}

#[derive(Template)]
#[template(path = "404_not_found.html")]
struct NotFound {
    context: Context,
    user: Option<User>,
}

async fn not_found(context: Context, session: Option<Session>) -> impl IntoResponse {
    NotFound {
        context,
        user: session.map(|s| s.user),
    }
}

#[derive(Deserialize)]
struct AuthenticateQuery {
    token: String,
    redirect_path: Option<String>,
}

#[allow(unreachable_code, unused)]
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

    let user: QueryResult<User> = users
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

async fn whoops_500(context: Context, session: Option<Session>) -> Response {
    let err = anyhow::anyhow!("Hold my beverage!");
    context.error(session, err.into())
}

async fn whoops_422(context: Context, session: Option<Session>) -> Response {
    let err = AppError::CsrfMismatch;
    context.error(session, err)
}

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
