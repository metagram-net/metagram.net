use askama::Template;
use axum::{
    extract::FromRef,
    http::{Request, StatusCode},
    response::{IntoResponse, IntoResponseParts, Response, ResponseParts},
    Router,
};
use axum_csrf::{CsrfConfig, CsrfToken};
use derivative::Derivative;
use sqlx::PgPool;
use std::net::SocketAddr;
use tokio::sync::watch;
use tower::ServiceBuilder;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;

pub mod models;
pub mod view_models;
pub use models::User;

pub mod auth;
use auth::Session;
pub use auth::{Auth, AuthN};

pub mod firehose;

mod filters;
pub mod jobs;
pub mod queue;
mod web;

const COMMIT_HASH: &str = include_str!(concat!(env!("OUT_DIR"), "/commit_hash"));
const BUILD_PROFILE: &str = include_str!(concat!(env!("OUT_DIR"), "/build_profile"));
const RAW_LICENSE_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/licenses.html"));

const SOURCE_URL: &str = "https://github.com/metagram-net/metagram.net";

pub struct PgConn(sqlx::pool::PoolConnection<sqlx::Postgres>);

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for PgConn
where
    S: Send + Sync,
    CsrfConfig: FromRef<S>,
    PgPool: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let pool = PgPool::from_ref(state);

        match pool.acquire().await {
            Ok(conn) => Ok(PgConn(conn)),
            Err(err) => {
                let context = Context::from_request_parts(parts, state).await.unwrap();
                Err(context.error(None, err.into()))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BaseUrl(url::Url);

pub struct ServerConfig {
    pub auth: auth::Auth,
    pub base_url: url::Url,
    pub cookie_key: cookie::Key,
    pub database_pool: PgPool,
}

pub struct Server {
    app: Router<()>,
}

#[derive(Clone, FromRef)]
pub struct AppState {
    base_url: BaseUrl,
    database_pool: PgPool,
    cookie_key: cookie::Key,
    auth: Auth,
    csrf_config: CsrfConfig,
}

impl Server {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let state = AppState {
            base_url: BaseUrl(config.base_url),
            database_pool: config.database_pool,
            cookie_key: config.cookie_key.clone(),
            auth: config.auth,
            csrf_config: CsrfConfig::new()
                .with_cookie_path("/")
                .with_secure(true)
                .with_http_only(true)
                .with_cookie_same_site(axum_csrf::SameSite::Strict)
                // There are two versions of cookie around, and axum-extra's is currently
                // newer than axum-csrf's. So convert the key from one to the other by re-parsing
                // the bytes.
                .with_key(Some(axum_csrf::Key::from(config.cookie_key.master()))),
        };

        let router = Router::new()
            .merge(web::router())
            .fallback(not_found)
            .with_state(state.clone())
            .nest_service("/dist", ServeDir::new("dist"));

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
            // This ordering is important! While processing the inbound request, auto_csrf_token
            // assumes that CsrfLayer has already extracted the authenticity token from the
            // cookies. When generating the outbound response, order doesn't matter. So keep
            // auto_csrf_token deeper in the middleware stack.
            //
            // Why do this on every request instead of just where it's needed? When the user is
            // logged in, every page (even error pages) contains a logout form that would require
            // doing this anyway.
            //
            // TODO: Could this become CsrfLayer's job?
            .layer(axum::middleware::from_fn_with_state(state, auto_csrf_token));

        Ok(Self { app })
    }

    pub async fn run(
        self,
        addr: SocketAddr,
        mut shutdown: watch::Receiver<bool>,
    ) -> hyper::Result<()> {
        tracing::info!("Listening on http://{}", addr);
        axum::Server::bind(&addr)
            .serve(self.app.into_make_service())
            .with_graceful_shutdown(async {
                // Either this is a legit shutdown signal or the sender disappeared. Either way,
                // we're done!
                let _ = shutdown.changed().await;
            })
            .await
    }
}

async fn auto_csrf_token<B: Send>(
    csrf_token: CsrfToken,
    req: Request<B>,
    next: axum::middleware::Next<B>,
) -> impl IntoResponse {
    (csrf_token, next.run(req).await)
}

#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("authenticity token mismatch")]
    CsrfMismatch,

    #[error(transparent)]
    StytchError(#[from] stytch::Error),

    #[error(transparent)]
    SqlxError(#[from] sqlx::Error),

    #[error(transparent)]
    Unhandled(#[from] anyhow::Error),
}

#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct Context {
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
            SqlxError(err) => {
                tracing::error!({ ?err }, "sqlx error");
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
impl<S> axum::extract::FromRequestParts<S> for Context
where
    S: Send + Sync,
    CsrfConfig: FromRef<S>,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let csrf_token = CsrfToken::from_request_parts(parts, state)
            .await
            .expect("layer: CsrfToken");

        let request_id = parts
            .headers
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
#[template(path = "errors/500_internal_server_error.html")]
struct InternalServerError {
    context: Context,
    user: Option<User>,
}

#[derive(Template)]
#[template(path = "errors/422_unprocessable_entity.html")]
struct UnprocessableEntity {
    context: Context,
    user: Option<User>,
}

#[derive(Template)]
#[template(path = "errors/404_not_found.html")]
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
