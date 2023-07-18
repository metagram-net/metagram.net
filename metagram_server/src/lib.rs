use askama::Template;
use axum::{
    extract::FromRef,
    http::Request,
    response::{IntoResponse, IntoResponseParts, ResponseParts},
    Router,
};
use axum_csrf::{CsrfConfig, CsrfLayer, CsrfToken};
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
    PgPool: FromRef<S>,
{
    type Rejection = web::Error;

    async fn from_request_parts(
        _parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let pool = PgPool::from_ref(state);

        let conn = pool.acquire().await?;

        Ok(PgConn(conn))
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
}

impl Server {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let state = AppState {
            base_url: BaseUrl(config.base_url),
            database_pool: config.database_pool,
            cookie_key: config.cookie_key.clone(),
            auth: config.auth,
        };

        let router = Router::new()
            .merge(web::router(state.clone()))
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

        let csrf_layer = CsrfLayer::new(
            CsrfConfig::new()
                .with_cookie_path("/")
                .with_secure(true)
                .with_http_only(true)
                .with_cookie_same_site(axum_csrf::SameSite::Strict)
                // There are two versions of cookie around, and axum-extra's is currently
                // newer than axum-csrf's. So convert the key from one to the other by re-parsing
                // the bytes.
                .with_key(Some(axum_csrf::Key::from(config.cookie_key.master()))),
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
            .layer(csrf_layer)
            // Why include the CSRF-protection token on every request instead of just where it's
            // needed?
            //
            // When the user is logged in, every page (even error pages) contains a logout
            // form that would require doing this anyway. If there's no logged-in user, they're
            // probably at the login page, which is also a form!
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

#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct Context {
    #[derivative(Debug = "ignore")]
    csrf_token: CsrfToken,

    // Computing csrf_token.authenticity_token() an expensive hash by design, but it only needs to
    // be computed once per response. Cache it here to speed up rendering for multi-form pages.
    #[derivative(Debug = "ignore")]
    authenticity_token: String,

    request_id: Option<String>,
}

impl Context {
    pub fn verify_csrf(&self, authenticity_token: &str) -> web::Result<()> {
        self.csrf_token
            .verify(authenticity_token)
            .map_err(|_| web::Error::CsrfMismatch {
                cookie: self.csrf_token.authenticity_token().unwrap_or_default(),
                form: authenticity_token.to_string(),
            })
    }
}

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for Context
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        if let Some(ctx) = parts.extensions.get::<Self>() {
            return Ok(ctx.clone());
        }

        let csrf_token = CsrfToken::from_request_parts(parts, state)
            .await
            .expect("layer: CsrfLayer");

        let request_id = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let authenticity_token = csrf_token
            .authenticity_token()
            .expect("hashing failure, internal server error");

        let ctx = Self {
            csrf_token,
            authenticity_token,
            request_id,
        };

        parts.extensions.insert(ctx.clone());
        Ok(ctx)
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
