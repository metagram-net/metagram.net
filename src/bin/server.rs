use async_trait::async_trait;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;

#[derive(Deserialize, Debug)]
struct Config {
    cookie_key: String,
    database_url: String,
    base_url: String,

    #[serde(default)]
    mock_auth: bool,

    #[serde(default, deserialize_with = "bool_from_string")]
    dev_logging: bool,

    stytch_env: stytch::Env,
    stytch_project_id: String,
    stytch_secret: String,
}

/// Deserialize bool from String with custom value mapping
fn bool_from_string<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_ref() {
        "1" => Ok(true),
        "TRUE" => Ok(true),
        "true" => Ok(true),

        "0" => Ok(false),
        "FALSE" => Ok(false),
        "false" => Ok(false),

        other => Err(serde::de::Error::invalid_value(
            serde::de::Unexpected::Str(other),
            &"1, TRUE, true, 0, FALSE, false",
        )),
    }
}

#[tokio::main]
async fn main() {
    let config = match envy::from_env::<Config>() {
        Ok(settings) => settings,
        Err(err) => panic!("{:#?}", err),
    };

    if config.dev_logging {
        tracing_subscriber::fmt().pretty().init();
    } else {
        tracing_subscriber::fmt().json().init();
    }

    let base_url = url::Url::parse(&config.base_url).expect("BASE_URL should be a valid URL");

    let cookie_key = {
        let key = base64::decode(config.cookie_key).expect("COOKIE_KEY should be valid base64");
        cookie::Key::from(&key)
    };

    let auth: metagram::Auth = {
        if config.mock_auth {
            tracing::warn!("Using the mock authentication provider");
            Arc::new(mock_auth())
        } else {
            let stytch_config = stytch::Config {
                env: config.stytch_env,
                project_id: config.stytch_project_id,
                secret: config.stytch_secret,
            };

            let minutes = chrono::Duration::days(30)
                .num_minutes()
                .try_into()
                .expect("session duration should fit in u32");

            Arc::new(StytchAuth {
                client: stytch_config.client().expect("Stytch client"),
                base_url: base_url.clone(),
                session_duration_minutes: Some(minutes),
            })
        }
    };

    let database_pool = PgPoolOptions::new()
        .connect(&config.database_url)
        .await
        .expect("database_pool");

    let srv = metagram::Server::new(metagram::ServerConfig {
        auth,
        base_url,
        cookie_key,
        database_pool,
    })
    .await
    .unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    srv.run(addr, shutdown_signal()).await.unwrap();

    tracing::info!("Goodbye! ✌");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Signal received, starting graceful shutdown");
}

#[derive(Debug, Clone)]
struct StytchAuth {
    client: stytch::Client,
    base_url: url::Url,
    session_duration_minutes: Option<u32>,
}

#[async_trait]
impl metagram::AuthN for StytchAuth {
    async fn send_magic_link(
        &self,
        email: String,
        callback_path: String,
        // TODO: target_path: String // post-auth re-redirect
    ) -> stytch::Result<stytch::magic_links::email::SendResponse> {
        let url = self.base_url.join(&callback_path).expect("valid URL");

        let req = stytch::magic_links::email::SendRequest {
            email,
            login_magic_link_url: Some(url.to_string()),
            signup_magic_link_url: Some(url.to_string()),
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
            session_duration_minutes: self.session_duration_minutes,
            ..Default::default()
        };
        req.send(self.client.clone()).await
    }

    async fn authenticate_session(
        &self,
        token: String,
    ) -> stytch::Result<stytch::sessions::AuthenticateResponse> {
        let req = stytch::sessions::AuthenticateRequest {
            session_token: Some(token),
            session_duration_minutes: self.session_duration_minutes,
            ..Default::default()
        };
        req.send(self.client.clone()).await
    }

    async fn revoke_session(
        &self,
        token: String,
    ) -> stytch::Result<stytch::sessions::RevokeResponse> {
        let req = stytch::sessions::RevokeRequest {
            session_token: Some(token),
            ..Default::default()
        };
        req.send(self.client.clone()).await
    }
}

pub fn mock_auth() -> metagram::MockAuthN {
    use mockall::predicate as p;

    let mut mock = metagram::MockAuthN::new();

    mock.expect_send_magic_link()
        .with(p::eq("user@metagram.test".to_string()), p::always())
        .returning(|_, _| {
            Ok(stytch::magic_links::email::SendResponse {
                status_code: http::StatusCode::OK,
                request_id: "mock-request".to_string(),
                user_id: std::env::var("STYTCH_USER_ID").unwrap(),
                email_id: "".to_string(),
            })
        });

    mock.expect_authenticate_magic_link()
        .returning(|stytch_user_id| {
            Ok(stytch::magic_links::AuthenticateResponse {
                user_id: stytch_user_id.clone(),
                status_code: http::StatusCode::OK,
                request_id: "mock-request".to_string(),
                user: stytch::User {},
                session: Some(stytch::Session {
                    user_id: stytch_user_id,
                    session_id: "mock-session".to_string(),
                    authentication_factors: vec![],
                    started_at: chrono::Utc::now() - chrono::Duration::hours(1),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    last_accessed_at: chrono::Utc::now(),
                    attributes: stytch::Attributes {
                        ip_address: "".to_string(),
                        user_agent: "".to_string(),
                    },
                }),
                session_token: "mock-session-token".to_string(),
                session_jwt: String::new(),
            })
        });

    mock.expect_authenticate_session()
        .with(p::eq("mock-session-token".to_string()))
        .returning(|tok| {
            Ok(stytch::sessions::AuthenticateResponse {
                status_code: http::StatusCode::OK,
                request_id: "mock-request".to_string(),
                session: stytch::Session {
                    user_id: std::env::var("STYTCH_USER_ID").unwrap(),
                    session_id: "mock-session".to_string(),
                    authentication_factors: vec![],
                    started_at: chrono::Utc::now() - chrono::Duration::hours(1),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    last_accessed_at: chrono::Utc::now(),
                    attributes: stytch::Attributes {
                        ip_address: "".to_string(),
                        user_agent: "".to_string(),
                    },
                },
                session_token: Some(tok),
                session_jwt: String::new(),
            })
        });

    mock.expect_authenticate_session().returning(|_| {
        Err(stytch::Error::Response(stytch::ErrorResponse {
            status_code: http::StatusCode::UNAUTHORIZED,
            request_id: "mock-request".to_string(),
            error_type: "invalid_session_token".to_string(),
            error_message: "...".to_string(),
            error_url: "...".to_string(),
        }))
    });

    mock
}
