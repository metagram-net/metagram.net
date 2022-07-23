#[macro_use]
extern crate diesel;

use askama::Template;
use axum::{
    extract::{Extension, RequestParts},
    handler::Handler,
    http::{Request, StatusCode},
    response::{IntoResponse, IntoResponseParts, Response, ResponseParts},
    routing::get,
    Json, Router,
};
use axum_csrf::{CsrfLayer, CsrfToken};
use derivative::Derivative;
use diesel_async::{
    pooled_connection::deadpool::{self, Object, Pool},
    AsyncPgConnection,
};
use serde::Serialize;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;

mod models;
mod schema;
mod sql_types;

use models::User;

pub(crate) mod auth;
use auth::Session;
pub use auth::{Auth, AuthN, MockAuthN};

mod firehose;

type PgPool = Pool<AsyncPgConnection>;

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
    pub auth: auth::Auth,
}

pub struct Server {
    router: Router,
}

impl Server {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let router = Router::new()
            .route("/", get(index))
            .route("/.well-known/health-check", get(health_check))
            .merge(auth::router())
            .nest("/firehose", firehose::router())
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
            .layer(Extension::<auth::Auth>(config.auth))
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

async fn whoops_500(context: Context, session: Option<Session>) -> Response {
    let err = anyhow::anyhow!("Hold my beverage!");
    context.error(session, err.into())
}

async fn whoops_422(context: Context, session: Option<Session>) -> Response {
    let err = AppError::CsrfMismatch;
    context.error(session, err)
}
