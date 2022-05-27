use serde::{de::DeserializeOwned, Deserialize, Serialize};
use url::Url;

pub const LIVE: &'static str = "https://api.stytch.com/v1/";
pub const TEST: &'static str = "https://test.stytch.com/v1/";

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

    #[error(transparent)]
    UninitializedFieldError(#[from] derive_builder::UninitializedFieldError),
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

pub mod debug {
    use super::{send, Client, Result};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct WhoamiResponse {
        #[serde(with = "http_serde::status_code")]
        pub status_code: http::StatusCode,
        pub request_id: String,

        pub project_id: String,
    }

    pub async fn whoami(client: &Client) -> Result<WhoamiResponse> {
        let req = client.request(http::Method::GET, "debug/whoami")?;
        send(req).await
    }
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

    #[derive(Serialize, Deserialize, Debug, Clone, Default, derive_builder::Builder)]
    #[builder(
        default,
        setter(strip_option),
        build_fn(error = "crate::stytch::Error")
    )]
    pub struct AuthenticateRequest {
        #[builder(setter(into))]
        pub token: String,
        pub session_duration_minutes: Option<u32>,
        #[builder(setter(into))]
        pub session_token: Option<String>,
        #[builder(setter(into))]
        pub session_jwt: Option<String>,
    }

    impl AuthenticateRequest {
        pub fn new(token: impl Into<String>) -> AuthenticateRequestBuilder {
            AuthenticateRequestBuilder {
                token: Some(token.into()),
                ..Default::default()
            }
        }

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

    pub async fn authenticate(
        client: &Client,
        token: impl Into<String>,
    ) -> Result<AuthenticateResponse> {
        let req = AuthenticateRequest::new(token.into()).build()?;
        req.send(client).await
    }

    pub mod email {
        use crate::stytch::{Client, Result};
        use serde::{Deserialize, Serialize};

        #[derive(
            Serialize, Deserialize, Debug, Clone, Default, derive_builder::Builder, PartialEq, Eq,
        )]
        #[builder(
            default,
            setter(strip_option),
            build_fn(error = "crate::stytch::Error")
        )]
        pub struct SendRequest {
            #[builder(setter(into))]
            pub email: String,
            #[builder(setter(into))]
            pub login_magic_link_url: Option<String>,
            #[builder(setter(into))]
            pub signup_magic_link_url: Option<String>,
            pub login_expiration_minutes: Option<u32>,
            pub signup_expiration_minutes: Option<u32>,
        }

        impl SendRequest {
            pub fn new(email: impl Into<String>) -> SendRequestBuilder {
                SendRequestBuilder {
                    email: Some(email.into()),
                    ..Default::default()
                }
            }

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

        pub async fn send(client: &Client, email: impl Into<String>) -> Result<SendResponse> {
            let req = SendRequest::new(email.into()).build()?;
            req.send(client).await
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::stytch;

    #[test]
    fn simplest_request() -> anyhow::Result<()> {
        stytch::magic_links::email::SendRequest::new("user@example.test".to_string()).build()?;
        Ok(())
    }

    #[test]
    fn complex_request() -> anyhow::Result<()> {
        use stytch::magic_links::email::*;
        let req = SendRequest::new("user@example.test")
            .login_magic_link_url("https://example.test/authenticate")
            .signup_magic_link_url("https://example.test/authenticate")
            .login_expiration_minutes(30)
            .signup_expiration_minutes(1440)
            .build()?;

        assert_eq!(
            req,
            SendRequest {
                email: "user@example.test".to_string(),
                login_magic_link_url: Some("https://example.test/authenticate".to_string()),
                signup_magic_link_url: Some("https://example.test/authenticate".to_string()),
                login_expiration_minutes: Some(30),
                signup_expiration_minutes: Some(1440),
            }
        );
        Ok(())
    }
}
