use askama::Template;
use axum::{
    extract::Extension,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use hyper::Body;
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::env;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestId, RequestId},
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::{info, span, Level};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
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

    let db_url = must_env("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .connect(&db_url)
        .await
        .expect("database connection");

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<Body>| {
            // Extract _only_ the request ID header. DefaultMakeSpan dumps all the headers, which
            // is way too much info.
            let request_id = match req.headers().get("x-request-id") {
                Some(val) => val.to_str().unwrap_or(""),
                None => "",
            };
            span!(
                Level::INFO,
                "request",
                method = %req.method(),
                uri = %req.uri(),
                version = ?req.version(),
                %request_id,
            )
        })
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO));

    let app = Router::new()
        .route("/", get(index))
        .route("/.well-known/health-check", get(health_check))
        .layer(
            ServiceBuilder::new()
                // To have request IDs show up in traces, the tracing middleware has to be
                // _between_ the request_id ones.
                .set_x_request_id(MakeRequestUuid)
                .layer(trace_layer)
                .propagate_x_request_id(),
        )
        .layer(Extension::<PgPool>(pool));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));

    info!("Listening on http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[derive(Clone, Copy)]
struct MakeRequestUuid;

impl MakeRequestId for MakeRequestUuid {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let request_id = Uuid::new_v4().to_string().parse().unwrap();
        Some(RequestId::new(request_id))
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    name: String,
}

async fn index(Extension(pool): Extension<PgPool>) -> Index {
    let user: User = sqlx::query_as("select * from users where id = $1")
        .bind(uuid::Uuid::parse_str(&must_env("LOCAL_USER_ID")).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();

    dbg!(&user);

    return Index { name: user.email };
}

#[derive(Serialize)]
struct Health {
    status: String,
}

async fn health_check() -> impl IntoResponse {
    let health = Health {
        status: "Ok".to_string(),
    };
    (StatusCode::OK, Json(health))
}
