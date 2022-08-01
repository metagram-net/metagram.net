use async_trait::async_trait;
use diesel_async::{
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = match envy::from_env::<Config>() {
        Ok(settings) => settings,
        Err(err) => panic!("{:#?}", err),
    };

    let base_url = url::Url::parse(&config.base_url).expect("BASE_URL should be a valid URL");

    let cookie_key = {
        let key = base64::decode(config.cookie_key).expect("COOKIE_KEY should be valid base64");
        cookie::Key::from(&key)
    };

    let auth: firehose::Auth = {
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

            let minutes = chrono::Duration::days(30)
                .num_minutes()
                .try_into()
                .expect("session duration should fit in u32");

            Arc::new(StytchAuth {
                client: stytch_config.client().expect("Stytch client"),
                redirect_target: base_url.join("authenticate").expect("redirect_target"),
                session_duration_minutes: Some(minutes),
            })
        }
    };

    let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(config.database_url);
    let database_pool = Pool::builder(manager).build().expect("database_pool");

    let srv = firehose::Server::new(firehose::ServerConfig {
        auth,
        base_url,
        cookie_key,
        database_pool,
    })
    .await
    .unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
    srv.run(addr).await.unwrap();
}

#[derive(Debug, Clone)]
struct StytchAuth {
    client: stytch::Client,
    redirect_target: url::Url,
    session_duration_minutes: Option<u32>,
}

#[async_trait]
impl firehose::AuthN for StytchAuth {
    async fn send_magic_link(
        &self,
        email: String,
        // TODO: redirect_path: String
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

pub fn mock_auth() -> firehose::MockAuthN {
    use mockall::predicate as p;
    let mut mock = firehose::MockAuthN::new();
    mock.expect_send_magic_link()
        .with(p::eq("jdkaplan@metagram.net".to_string()))
        .returning(|_| {
            Ok(stytch::magic_links::email::SendResponse {
                status_code: http::StatusCode::OK,
                request_id: "mock-request".to_string(),
                user_id: std::env::var("STYTCH_USER_ID").expect("STYTCH_USER_ID"),
                email_id: "".to_string(),
            })
        });
    mock
}
