use anyhow;
use askama::Template;
use axum::{
    extract::{Extension, Form, Query},
    http::{Request, StatusCode},
    response::{IntoResponse, IntoResponseParts, Redirect, Response, ResponseParts},
    routing::{get, post},
    Json, Router,
};
use axum_csrf::{CsrfLayer, CsrfToken};
use axum_extra::extract::PrivateCookieJar;
use cookie::Cookie;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::env;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestId, RequestId},
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;
use uuid::Uuid;

const SESSION_COOKIE_NAME: &str = "firehose_session";

#[allow(unused)]
#[derive(Debug, Clone, sqlx::FromRow)]
struct User {
    id: uuid::Uuid,
    email: String,
    encrypted_password: String,
    reset_password_token: Option<String>,
    reset_password_sent_at: Option<chrono::NaiveDateTime>,
    remember_created_at: Option<chrono::NaiveDateTime>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    confirmation_token: Option<String>,
    confirmed_at: Option<chrono::NaiveDateTime>,
    confirmation_sent_at: Option<chrono::NaiveDateTime>,
    unconfirmed_email: Option<String>,
}

fn must_env(var: &str) -> String {
    env::var(var).expect(var)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cookie_key = {
        let val = must_env("COOKIE_KEY");
        let key = base64::decode(val).expect("COOKIE_KEY should be valid base64");
        cookie::Key::from(&key)
    };

    let database_url = must_env("DATABASE_URL");

    let config = Config {
        cookie_key,
        database_url,
    };

    let srv = Server::new(config).await.unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    srv.run(addr).await.unwrap();
}

struct Config {
    cookie_key: cookie::Key,
    database_url: String,
}

struct Server {
    router: Router,
}

impl Server {
    async fn new(config: Config) -> anyhow::Result<Self> {
        let pool = { PgPoolOptions::new().connect(&config.database_url).await? };

        let router = Router::new()
            .route("/", get(index))
            .route("/login", get(login).post(login_form))
            .route("/logout", post(logout))
            .route("/authenticate", get(authenticate))
            .route("/.well-known/health-check", get(health_check));
        // TODO: .fallback(not_found) handler

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
                    .set_x_request_id(MakeRequestUuid)
                    .layer(trace_layer)
                    .propagate_x_request_id(),
            )
            .layer(Extension::<PgPool>(pool.clone()))
            .layer(Extension::<DebugAuth>(DebugAuth { pool }))
            .layer(Extension::<cookie::Key>(config.cookie_key.clone()))
            .layer(CsrfLayer::build().key(config.cookie_key).finish());

        Ok(Self { router: app })
    }

    async fn run(self, addr: SocketAddr) -> hyper::Result<()> {
        tracing::info!("Listening on http://{}", addr);
        axum::Server::bind(&addr)
            .serve(self.router.into_make_service())
            .await
    }
}

#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("authenticity token mismatch")]
    CsrfMismatch,

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("{:?}", self);

        use AppError::*;
        match self {
            CsrfMismatch => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
            Internal(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

#[derive(Clone, Copy)]
struct MakeRequestUuid;

impl MakeRequestId for MakeRequestUuid {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let request_id = Uuid::new_v4().to_string().parse().unwrap();
        Some(RequestId::new(request_id))
    }
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

// TODO: Context -> Session and make everything return (Session, impl IntoResponse)

#[derive(Clone)]
struct Context {
    user: Option<User>,
    csrf_token: CsrfToken,
}

#[axum::async_trait]
impl<B> axum::extract::FromRequest<B> for Context
where
    B: Send,
{
    type Rejection = AppError;

    async fn from_request(
        req: &mut axum::extract::RequestParts<B>,
    ) -> Result<Self, Self::Rejection> {
        let pool: Extension<PgPool> = Extension::from_request(req)
            .await
            .expect("extension: PgPool");
        let cookies: PrivateCookieJar<cookie::Key> = PrivateCookieJar::from_request(req)
            .await
            .expect("PrivateCookieJar");
        let csrf_token = CsrfToken::from_request(req)
            .await
            .expect("layer: CsrfToken");

        let user_id = cookies.get(SESSION_COOKIE_NAME).and_then(|cookie| {
            let val = cookie.value();
            uuid::Uuid::parse_str(val).ok()
        });

        let user: sqlx::Result<User> = sqlx::query_as("select * from users where id = $1")
            .bind(user_id)
            .fetch_one(&*pool)
            .await;

        if let Err(ref err) = user {
            tracing::error!({ ?user_id, ?err }, "find user");
        }

        Ok(Self {
            user: user.ok(),
            csrf_token,
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
}

async fn index(context: Context) -> impl IntoResponse {
    (context.clone(), Index { context })
}

#[derive(Template)]
#[template(path = "login.html")]
struct Login {
    context: Context,
}

async fn login(context: Context) -> impl IntoResponse {
    (context.clone(), Login { context })
}

#[derive(Deserialize, Debug)]
struct LoginForm {
    authenticity_token: String,
    email: String,
}

async fn login_form(
    context: Context,
    Extension(auth): Extension<DebugAuth>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    tracing::debug!("form: {:?}", form);
    if let Err(_) = context.csrf_token.verify(&form.authenticity_token) {
        return Err(AppError::CsrfMismatch);
    }

    let res = auth.send_challenge(&form.email).await;
    match res {
        Ok(id) => {
            tracing::info!("Sent login link to user {}", id);
            Ok((
                context.clone(),
                LoginConfirmation {
                    context,
                    email: form.email,
                },
            ))
        }
        Err(err) => {
            tracing::error!("Could not send login link: {:?}", err);
            // TODO: if user error, 400 instead
            Err(AppError::Internal(err))
        }
    }
}

#[derive(Template)]
#[template(path = "login_confirmation.html")]
struct LoginConfirmation {
    context: Context,
    email: String,
}

#[derive(Template)]
#[template(path = "500_internal_server_error.html")]
struct InternalServerError {
    context: Context,
}

#[derive(Deserialize)]
struct AuthenticateQuery {
    token: String,
}

// TODO: Handle deserialization failure

async fn authenticate(
    Extension(auth): Extension<DebugAuth>,
    cookies: PrivateCookieJar,
    Query(query): Query<AuthenticateQuery>,
) -> Result<impl IntoResponse, AppError> {
    let user = auth.authenticate_challenge(&query.token).await?;
    tracing::info!("Successfully authenticated token for user {}", user.id);

    let cookies = cookies.add(
        Cookie::build(SESSION_COOKIE_NAME, user.id.to_string())
            .permanent()
            .secure(true)
            .finish(),
    );

    // TODO: Redirect back to intended page.
    Ok((cookies, Redirect::to("/")))
}

#[derive(Deserialize, Debug)]
struct LogoutForm {
    authenticity_token: String,
}

async fn logout(
    context: Context,
    cookies: PrivateCookieJar,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    tracing::debug!("form: {:?}", form);
    if let Err(_) = context.csrf_token.verify(&form.authenticity_token) {
        return Err(AppError::CsrfMismatch);
    }

    let cookies = cookies.remove(Cookie::new(SESSION_COOKIE_NAME, ""));

    // TODO: Revoke session

    Ok((cookies, Redirect::to("/")))
}

#[derive(Clone)]
struct DebugAuth {
    pool: PgPool,
}

impl DebugAuth {
    async fn send_challenge(self, email_address: &str) -> anyhow::Result<uuid::Uuid> {
        let user: User = sqlx::query_as("select * from users where email = $1")
            .bind(email_address)
            .fetch_one(&self.pool)
            .await?;
        Ok(user.id)
    }

    // 2d15aa0b-5bd9-4dea-ab9f-5ba3b0a913c0
    async fn authenticate_challenge(self, id: &str) -> anyhow::Result<User> {
        let user: User = sqlx::query_as("select * from users where id = $1")
            .bind(uuid::Uuid::parse_str(id)?)
            .fetch_one(&self.pool)
            .await?;
        Ok(user)
    }
}
