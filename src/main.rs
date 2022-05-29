use askama::Template;
use axum::{
    extract::{Extension, Form, Query, RequestParts},
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

mod stytch;

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

    let stytch_config = stytch::Config {
        env: stytch::TEST.to_string(),
        project_id: must_env("STYTCH_PROJECT_ID"),
        secret: must_env("STYTCH_SECRET"),
    };

    let config = Config {
        stytch_config,
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
    stytch_config: stytch::Config,
}

struct Server {
    router: Router,
}

impl Server {
    async fn new(config: Config) -> anyhow::Result<Self> {
        let pool = { PgPoolOptions::new().connect(&config.database_url).await? };

        let stytch_client = config.stytch_config.client()?;

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
            .layer(Extension::<PgPool>(pool))
            .layer(Extension::<cookie::Key>(config.cookie_key.clone()))
            .layer(Extension::<stytch::Client>(stytch_client))
            // This ordering is important! While processing the inbound request, auto_csrf_token
            // assumes that CsrfLayer has already extracted the authenticity token from the
            // cookies. When generating the outbound response, order doesn't matter. So keep
            // auto_csrf_token deeper in the middleware stack.
            .layer(axum::middleware::from_fn(auto_csrf_token))
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
    Unhandled(#[from] anyhow::Error),
    // TODO: Define these variants that render real templates:
    // TODO: InternalServerError(Context)
    // TODO: NotFound(Context)
    // TODO: UnprocessableEntity(Context)
    // TODO: ...
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("{:?}", self);

        use AppError::*;
        match self {
            CsrfMismatch => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
            StytchError(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Unhandled(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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

        // TODO: Stytch user ID?
        let user: sqlx::Result<User> = sqlx::query_as("select * from users where id = $1")
            .bind(user_id)
            .fetch_one(&*pool)
            .await;

        if let Err(ref err) = user {
            tracing::info!({ ?user_id, ?err }, "could not find user");
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
    Index { context }
}

#[derive(Template)]
#[template(path = "login.html")]
struct Login {
    context: Context,
}

async fn login(context: Context) -> impl IntoResponse {
    Login { context }
}

#[derive(Deserialize, Debug)]
struct LoginForm {
    authenticity_token: String,
    email: String,
}

async fn login_form(
    context: Context,
    Extension(auth): Extension<stytch::Client>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(AppError::CsrfMismatch);
    }

    let res = stytch::magic_links::email::SendRequest {
        email: form.email.clone(),
        // TODO: configurable base URL
        login_magic_link_url: Some("http://localhost:8000/authenticate".to_string()),
        signup_magic_link_url: Some("http://localhost:8000/authenticate".to_string()),
        ..Default::default()
    }
    .send(&auth)
    .await?;

    tracing::info!("Sent login link to user {}", res.user_id);
    Ok(LoginConfirmation {
        context,
        email: form.email,
    })
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
    Extension(auth): Extension<stytch::Client>,
    Extension(pool): Extension<PgPool>,
    cookies: PrivateCookieJar,
    Query(query): Query<AuthenticateQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = stytch::magic_links::AuthenticateRequest {
        token: query.token,
        ..Default::default()
    }
    .send(&auth)
    .await?;
    tracing::info!("Successfully authenticated token for user {}", res.user_id);

    let user: sqlx::Result<User> = sqlx::query_as("select * from users where stytch_user_id = $1")
        .bind(res.user_id.clone())
        .fetch_one(&pool)
        .await;

    match user {
        Ok(user) => {
            // TODO: Store the Stytch session token instead of local user ID
            let cookies = cookies.add(
                Cookie::build(SESSION_COOKIE_NAME, user.id.to_string())
                    .permanent()
                    .secure(true)
                    .finish(),
            );

            // TODO: Redirect back to intended page.
            Ok((cookies, Redirect::to("/")).into_response())
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
    cookies: PrivateCookieJar,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(AppError::CsrfMismatch);
    }

    let cookies = cookies.remove(Cookie::new(SESSION_COOKIE_NAME, ""));

    // TODO: Revoke session

    Ok((cookies, Redirect::to("/")))
}
