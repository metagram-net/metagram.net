use async_trait::async_trait;
use derivative::Derivative;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use url::Url;

const LIVE_URL: &str = "https://api.stytch.com/v1/";
const TEST_URL: &str = "https://test.stytch.com/v1/";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(from = "String")]
pub enum Env {
    Live,
    Test,
    Dev(String),
}

impl From<String> for Env {
    fn from(s: String) -> Self {
        return match s.as_str() {
            "live" => Env::Live,
            "Live" => Env::Live,
            "LIVE" => Env::Live,
            "test" => Env::Test,
            "Test" => Env::Test,
            "TEST" => Env::Test,
            _ => Env::Dev(s),
        };
    }
}

impl Env {
    fn base_url(self) -> std::result::Result<Url, url::ParseError> {
        match self {
            Env::Live => Url::parse(LIVE_URL),
            Env::Test => Url::parse(TEST_URL),
            Env::Dev(url) => {
                // The trailing slash is significant in the base URL. Without it, any later joins
                // would drop the last path segment.
                if url.ends_with('/') {
                    Url::parse(&url)
                } else {
                    Url::parse(&(url + "/"))
                }
            }
        }
    }
}

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

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Config {
    pub env: Env,
    pub project_id: String,
    pub secret: String,
}

impl Config {
    pub fn client(self) -> Result<Client> {
        Client::new(self)
    }
}

#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct Client {
    #[derivative(Debug = "ignore")]
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

        Ok(Self {
            client,
            base_url: config.env.base_url()?,
        })
    }
}

#[async_trait]
impl Sender for Client {
    fn request(&self, method: http::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.base_url.join(path)?;
        Ok(self.client.request(method, url))
    }

    async fn send<T>(&self, req: reqwest::RequestBuilder) -> Result<T>
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

type Timestamp = chrono::DateTime<chrono::Utc>;

// TODO: User
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct User {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub session_id: String,
    pub user_id: String,

    pub authentication_factors: Vec<AuthenticationFactor>,

    pub started_at: Timestamp,
    pub expires_at: Timestamp,
    pub last_accessed_at: Timestamp,

    pub attributes: Attributes,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationFactor {
    pub delivery_method: String,
    pub r#type: String,
    pub last_authenticated_at: Timestamp,

    #[serde(flatten)]
    factor: Factor,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Factor {
    #[serde(rename = "email_factor")]
    Email {
        #[serde(rename = "email_id")]
        id: String,
        #[serde(rename = "email_address")]
        address: String,
    },
    #[serde(rename = "phone_number_factor")]
    PhoneNumber {
        #[serde(rename = "phone_id")]
        id: String,
        #[serde(rename = "phone_number")]
        number: String,
    },
    // TODO: Fill in other factor variants
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    ip_address: String,
    user_agent: String,
}

#[async_trait]
pub trait Sender {
    /// Start building a request with the method and path.
    fn request(&self, method: http::Method, path: &str) -> Result<reqwest::RequestBuilder>;

    /// Send the built request and deserialize the response.
    async fn send<T>(&self, req: reqwest::RequestBuilder) -> Result<T>
    where
        T: DeserializeOwned + std::fmt::Debug;
}

macro_rules! route {
    ( $method:expr, $path:literal, $Req:ty, $Res:ty ) => {
        impl $Req {
            pub async fn send(self, client: impl crate::stytch::Sender) -> Result<$Res> {
                let req = client.request($method, $path)?.json(&self);
                client.send(req).await
            }
        }
    };
}

pub mod magic_links {
    use super::Result;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct AuthenticateRequest {
        pub token: String,
        pub session_duration_minutes: Option<u32>,
        pub session_token: Option<String>,
        pub session_jwt: Option<String>,
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

    route!(
        http::Method::POST,
        "magic_links/authenticate",
        AuthenticateRequest,
        AuthenticateResponse
    );

    pub mod email {
        use crate::stytch::Result;
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, Debug, Clone, Default)]
        pub struct SendRequest {
            pub email: String,
            pub login_magic_link_url: Option<String>,
            pub signup_magic_link_url: Option<String>,
            pub login_expiration_minutes: Option<u32>,
            pub signup_expiration_minutes: Option<u32>,
        }

        #[derive(Serialize, Deserialize, Debug, Clone)]
        pub struct SendResponse {
            #[serde(with = "http_serde::status_code")]
            pub status_code: http::StatusCode,
            pub request_id: String,

            pub user_id: String,
            pub email_id: String,
        }

        route!(
            http::Method::POST,
            "magic_links/email/send",
            SendRequest,
            SendResponse
        );
    }
}

pub mod sessions {
    use super::Result;
    use serde::{Deserialize, Serialize};
    use serde_with::{serde_as, NoneAsEmptyString};

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct AuthenticateRequest {
        pub session_duration_minutes: Option<u32>,
        pub session_token: Option<String>,
        pub session_jwt: Option<String>,
    }

    #[serde_as]
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct AuthenticateResponse {
        #[serde(with = "http_serde::status_code")]
        pub status_code: http::StatusCode,
        pub request_id: String,

        pub session: crate::stytch::Session,
        #[serde_as(as = "NoneAsEmptyString")]
        pub session_token: Option<String>,
        pub session_jwt: String,
    }

    route!(
        http::Method::POST,
        "sessions/authenticate",
        AuthenticateRequest,
        AuthenticateResponse
    );

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct RevokeRequest {
        pub session_id: Option<String>,
        pub session_token: Option<String>,
        pub session_jwt: Option<String>,
    }

    #[serde_as]
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct RevokeResponse {
        #[serde(with = "http_serde::status_code")]
        pub status_code: http::StatusCode,
        pub request_id: String,
    }

    route!(
        http::Method::POST,
        "sessions/revoke",
        RevokeRequest,
        RevokeResponse
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(s: &str) -> anyhow::Result<Timestamp> {
        Ok(chrono::DateTime::parse_from_rfc3339(s)?.with_timezone(&chrono::Utc))
    }

    #[test]
    fn deserialize_session() -> anyhow::Result<()> {
        let data = r#"
{
  "attributes": {
    "ip_address": "203.0.113.1",
    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36"
  },
  "authentication_factors": [
    {
      "delivery_method": "email",
      "email_factor": {
        "email_address": "sandbox@stytch.com",
        "email_id": "email-test-81bf03a8-86e1-4d95-bd44-bb3495224953"
      },
      "last_authenticated_at": "2021-08-09T07:41:52Z",
      "type": "magic_link"
    }
  ],
  "expires_at": "2021-08-10T07:41:52Z",
  "last_accessed_at": "2021-08-09T07:41:52Z",
  "session_id": "session-test-fe6c042b-6286-479f-8a4f-b046a6c46509",
  "started_at": "2021-08-09T07:41:52Z",
  "user_id": "user-test-16d9ba61-97a1-4ba4-9720-b03761dc50c6"
}
        "#;
        let session: Session = serde_json::from_str(data)?;

        let expected = Session {
            attributes: Attributes{
                ip_address: "203.0.113.1".to_string(),
                user_agent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36".to_string()
            },
            authentication_factors: vec![
                AuthenticationFactor{
                    delivery_method: "email".to_string(),
                    factor: Factor::Email{
                        address: "sandbox@stytch.com".to_string(),
                        id: "email-test-81bf03a8-86e1-4d95-bd44-bb3495224953".to_string()
                    },
                    last_authenticated_at: timestamp("2021-08-09T07:41:52Z")?,
                    r#type: "magic_link".to_string()
                }
            ],
            expires_at: timestamp("2021-08-10T07:41:52Z")?,
            last_accessed_at: timestamp("2021-08-09T07:41:52Z")?,
            session_id: "session-test-fe6c042b-6286-479f-8a4f-b046a6c46509".to_string(),
            started_at: timestamp("2021-08-09T07:41:52Z")?,
            user_id: "user-test-16d9ba61-97a1-4ba4-9720-b03761dc50c6".to_string(),
};
        assert_eq!(session, expected);
        Ok(())
    }
}
