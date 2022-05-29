// Almost all `new` methods are (intentionally) entry points to builders that allow sending the
// request directly. So this lint is too noisy within this module.
#![allow(clippy::new_ret_no_self)]

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use url::Url;

pub const LIVE: &str = "https://api.stytch.com/v1/";
pub const TEST: &str = "https://test.stytch.com/v1/";

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("{0:?}")]
    Response(ErrorResponse),

    #[error(transparent)]
    InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),

    #[error(transparent)]
    InvalidUrl(#[from] url::ParseError),

    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Config {
    pub env: String,
    pub project_id: String,
    pub secret: String,
}

impl Config {
    pub fn client(self) -> Result<Client> {
        Client::new(self)
    }
}

#[derive(Clone)]
pub struct Client {
    client: reqwest::Client,
    base_url: Url,
}

impl Client {
    pub fn new(config: Config) -> Result<Self> {
        let mut headers = http::header::HeaderMap::new();
        let encoded = base64::encode(format!("{}:{}", config.project_id, config.secret));
        let basic_auth = format!("Basic {}", encoded).parse::<http::header::HeaderValue>()?;
        headers.insert(http::header::AUTHORIZATION, basic_auth);

        let client = reqwest::Client::builder()
            // TODO: .user_agent()
            .default_headers(headers)
            .build()?;

        // The trailing slash is significant in the base URL. Without it, any later joins would
        // drop the last path segment.
        let env = if config.env.ends_with('/') {
            config.env
        } else {
            config.env + "/"
        };
        let base_url = Url::parse(&env)?;

        Ok(Self { client, base_url })
    }

    pub fn request(&self, method: http::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.base_url.join(path)?;
        Ok(self.client.request(method, url))
    }
}

pub async fn send<T>(req: reqwest::RequestBuilder) -> Result<T>
where
    T: DeserializeOwned + std::fmt::Debug,
{
    tracing::debug!({ req = ?req }, "send Stytch request");
    let res = req.send().await?;
    if res.status().is_success() {
        let body = res.json().await?;
        tracing::debug!({ ?body }, "Stytch response success");
        Ok(body)
    } else {
        let err = res.json::<ErrorResponse>().await?;
        tracing::debug!({ ?err }, "Stytch response error");
        Err(Error::Response(err))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    #[serde(with = "http_serde::status_code")]
    pub status_code: http::StatusCode,
    pub request_id: String,

    pub error_type: String,
    pub error_message: String,
    pub error_url: String,
}

// TODO: User
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {}

// TODO: Session
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Session {}

pub mod magic_links {
    use super::{Client, Result};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct AuthenticateRequest {
        pub token: String,
        pub session_duration_minutes: Option<u32>,
        pub session_token: Option<String>,
        pub session_jwt: Option<String>,
    }

    impl AuthenticateRequest {
        // TODO: Derive this with a macro!
        pub async fn send(self, client: &Client) -> Result<AuthenticateResponse> {
            let req = client
                .request(http::Method::POST, "magic_links/authenticate")?
                .json(&self);
            crate::stytch::send(req).await
        }
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct AuthenticateResponse {
        #[serde(with = "http_serde::status_code")]
        pub status_code: http::StatusCode,
        pub request_id: String,

        pub user_id: String,
        pub user: crate::stytch::User,
        pub session: Option<crate::stytch::Session>,
        pub session_token: String,
        pub session_jwt: String,
    }

    pub mod email {
        use crate::stytch::{Client, Result};
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, Debug, Clone, Default)]
        pub struct SendRequest {
            pub email: String,
            pub login_magic_link_url: Option<String>,
            pub signup_magic_link_url: Option<String>,
            pub login_expiration_minutes: Option<u32>,
            pub signup_expiration_minutes: Option<u32>,
        }

        impl SendRequest {
            // TODO: Derive this with a macro!
            pub async fn send(self, client: &Client) -> Result<SendResponse> {
                let req = client
                    .request(http::Method::POST, "magic_links/email/send")?
                    .json(&self);
                crate::stytch::send(req).await
            }
        }

        #[derive(Serialize, Deserialize, Debug, Clone)]
        pub struct SendResponse {
            #[serde(with = "http_serde::status_code")]
            pub status_code: http::StatusCode,
            pub request_id: String,

            pub user_id: String,
            pub email_id: String,
        }
    }
}
