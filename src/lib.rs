use askama::Template;
use axum::{
    extract::{Extension, RequestParts},
    handler::Handler,
    http::{Request, StatusCode},
    response::{IntoResponse, IntoResponseParts, Response, ResponseParts},
    Router,
};
use axum_csrf::{CsrfLayer, CsrfToken};
use axum_extra::routing::SpaRouter;
use derivative::Derivative;
use sqlx::PgPool;
use std::future::Future;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;

pub mod models;
pub mod view_models;
pub use models::User;

pub mod auth;
use auth::Session;
pub use auth::{Auth, AuthN, MockAuthN};

pub mod firehose;

mod controllers;
mod filters;
mod routes;

pub struct PgConn(sqlx::pool::PoolConnection<sqlx::Postgres>);

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

        match pool.acquire().await {
            Ok(conn) => Ok(PgConn(conn)),
            Err(err) => {
                let context = Context::from_request(req).await.unwrap();
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
    router: Router,
}

impl Server {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let router = routes::build()
            // Apparently the SPA router is the easiest way to serve assets at a nested route.
            .merge(SpaRouter::new("/dist", "dist"))
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
            .layer(Extension::<BaseUrl>(BaseUrl(config.base_url)))
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

    pub async fn run(
        self,
        addr: SocketAddr,
        signal: impl Future<Output = ()>,
    ) -> hyper::Result<()> {
        tracing::info!("Listening on http://{}", addr);
        axum::Server::bind(&addr)
            .serve(self.router.into_make_service())
            .with_graceful_shutdown(signal)
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
