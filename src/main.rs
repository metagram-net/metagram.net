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
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;

mod stytch;

const SESSION_COOKIE_NAME: &str = "firehose_session";

#[derive(Deserialize, Debug)]
struct Config {
    cookie_key: String,
    database_url: String,
    base_url: String,

    #[serde(default)]
    mock_auth: bool,

    stytch_env: stytch::Env,
    stytch_project_id: String,
    stytch_secret: String,
}

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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = match envy::from_env::<Config>() {
        Ok(settings) => settings,
        Err(err) => panic!("{:#?}", err),
    };

    let cookie_key = {
        let key = base64::decode(config.cookie_key).expect("COOKIE_KEY should be valid base64");
        cookie::Key::from(&key)
    };

    let auth: Auth = {
        if config.mock_auth {
            Arc::new(mock_auth())
        } else {
            let stytch_config = stytch::Config {
                env: config.stytch_env,
                project_id: config.stytch_project_id,
                secret: config.stytch_secret,
            };

            let base_url: url::Url = config
                .base_url
                .parse()
                .expect("BASE_URL should be a valid URL");

            Arc::new(StytchAuth {
                client: stytch_config.client().expect("Stytch client"),
                redirect_target: base_url.join("authenticate").expect("redirect_target"),
            })
        }
    };

    let database_pool = PgPoolOptions::new()
        .connect(&config.database_url)
        .await
        .expect("database_pool");

    let srv = Server::new(ServerConfig {
        auth,
        cookie_key,
        database_pool,
    })
    .await
    .unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    srv.run(addr).await.unwrap();
}

struct ServerConfig {
    cookie_key: cookie::Key,
    database_pool: PgPool,
    auth: Auth,
}

struct Server {
    router: Router,
}

#[derive(Debug, Clone)]
struct BaseUrl(url::Url);

#[derive(Debug, Clone)]
struct StytchAuth {
    client: stytch::Client,
    redirect_target: url::Url,
}

type Auth = Arc<dyn AuthN + Send + Sync>;

impl Server {
    async fn new(config: ServerConfig) -> anyhow::Result<Self> {
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
    user: Option<User>,
    #[derivative(Debug = "ignore")]
    csrf_token: CsrfToken,
    request_id: Option<String>,
}

impl Context {
    fn error(self, err: AppError) -> Response {
        tracing::error!("{:?}", err);

        use AppError::*;
        match err {
            CsrfMismatch => (
                StatusCode::UNPROCESSABLE_ENTITY,
                UnprocessableEntity { context: self },
            )
                .into_response(),
            StytchError(err) => {
                tracing::error!({ ?err }, "stytch error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InternalServerError { context: self },
                )
                    .into_response()
            }
            Unhandled(err) => {
                tracing::error!({ ?err }, "unhandled error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    InternalServerError { context: self },
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

        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Ok(Self {
            user: user.ok(),
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
    Extension(auth): Extension<Auth>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    if context.csrf_token.verify(&form.authenticity_token).is_err() {
        return Err(context.error(AppError::CsrfMismatch));
    }

    let res = match auth.send_magic_link(form.email.clone()).await {
        Ok(res) => res,
        Err(err) => return Err(context.error(err.into())),
    };

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

#[derive(Template)]
#[template(path = "422_unprocessable_entity.html")]
struct UnprocessableEntity {
    context: Context,
}

#[derive(Template)]
#[template(path = "404_not_found.html")]
struct NotFound {
    context: Context,
}

async fn not_found(context: Context) -> impl IntoResponse {
    NotFound { context }
}

#[derive(Deserialize)]
struct AuthenticateQuery {
    token: String,
}

async fn authenticate(
    context: Context,
    cookies: PrivateCookieJar,
    Extension(pool): Extension<PgPool>,
    Extension(auth): Extension<Auth>,
    Query(query): Query<AuthenticateQuery>,
) -> impl IntoResponse {
    let res = match auth.authenticate_magic_link(query.token).await {
        Ok(res) => res,
        Err(err) => return Err(context.error(err.into())),
    };
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
        return Err(context.error(AppError::CsrfMismatch));
    }

    let cookies = cookies.remove(Cookie::new(SESSION_COOKIE_NAME, ""));

    // TODO: Revoke session

    Ok((cookies, Redirect::to("/")))
}

async fn whoops_500(context: Context) -> Response {
    let err = anyhow::anyhow!("Hold my beverage!");
    context.error(err.into())
}

async fn whoops_422(context: Context) -> Response {
    let err = AppError::CsrfMismatch;
    context.error(err)
}

#[mockall::automock]
#[async_trait]
trait AuthN {
    async fn send_magic_link(
        &self,
        email: String,
    ) -> stytch::Result<stytch::magic_links::email::SendResponse>;

    async fn authenticate_magic_link(
        &self,
        token: String,
    ) -> stytch::Result<stytch::magic_links::AuthenticateResponse>;
}

#[async_trait]
impl AuthN for StytchAuth {
    async fn send_magic_link(
        &self,
        email: String,
    ) -> stytch::Result<stytch::magic_links::email::SendResponse> {
        let req = stytch::magic_links::email::SendRequest {
            email,
            login_magic_link_url: Some(self.redirect_target.to_string()),
            signup_magic_link_url: Some(self.redirect_target.to_string()),
            ..Default::default()
        };
        req.send(self.client.clone()).await
    }

    async fn authenticate_magic_link(
        &self,
        token: String,
    ) -> stytch::Result<stytch::magic_links::AuthenticateResponse> {
        let req = stytch::magic_links::AuthenticateRequest {
            token,
            ..Default::default()
        };
        req.send(self.client.clone()).await
    }
}

fn mock_auth() -> MockAuthN {
    use mockall::predicate as p;
    let mut mock = MockAuthN::new();
    mock.expect_send_magic_link()
        .with(p::eq("jdkaplan@metagram.net".to_string()))
        .returning(|_| {
            Ok(stytch::magic_links::email::SendResponse {
                status_code: http::StatusCode::OK,
                request_id: "mock-request".to_string(),
                user_id: "74fba03a-0c9a-4f86-b255-549e479821cf".to_string(),
                email_id: "todo!".to_string(),
            })
        });
    mock
}
