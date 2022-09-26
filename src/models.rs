use chrono::NaiveDateTime as Timestamp;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use serde::{Deserialize, Serialize};
use sqlx::{Decode, FromRow, Type};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, Decode)]
pub struct User {
    pub id: Uuid,
    pub stytch_user_id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "drop_status", rename_all = "lowercase")]
pub enum DropStatus {
    Unread,
    Read,
    Saved,
}

impl std::fmt::Display for DropStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Self::Unread => "unread",
            Self::Read => "read",
            Self::Saved => "saved",
        };
        write!(f, "{}", s)
    }
}

impl Distribution<DropStatus> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> DropStatus {
        match rng.gen_range(0..=2) {
            0 => DropStatus::Unread,
            1 => DropStatus::Read,
            _ => DropStatus::Saved,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Drop {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: Option<String>,
    pub url: String,
    pub status: DropStatus,
    pub moved_at: Timestamp,
    // TODO: pub hydrant_id: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Drop {
    pub fn domain(&self) -> Option<String> {
        use addr::psl::parse_domain_name;
        use url::Url;

        let url = self.url.clone();
        let domain = match Url::parse(&url) {
            Ok(url) => url.domain().map(|s| s.to_string()).unwrap_or_default(),
            Err(err) => {
                tracing::error!({ ?err, ?url }, "unparseable URL");
                return None;
            }
        };

        match parse_domain_name(&domain) {
            Ok(domain) => Some(domain.to_string()),
            Err(err) => {
                tracing::error!({ ?err, ?url }, "URL without valid domain");
                None
            }
        }
    }

    pub fn display_text(&self) -> String {
        self.title.as_ref().unwrap_or(&self.url).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, Decode)]
pub struct Tag {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Tag {
    pub const DEFAULT_COLOR: &'static str = "#EEEEEE";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, Decode)]
pub struct DropTag {
    pub id: Uuid,
    pub drop_id: Uuid,
    pub tag_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, Decode)]
pub struct Stream {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub tag_ids: Vec<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Hydrant {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub url: String,
    pub active: bool,
    pub tag_ids: Vec<Uuid>,
    pub fetched_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn render_domain() {
        let d = drop(None, "https://example.net/something".to_string());
        assert_eq!(d.domain(), Some("example.net".to_string()));

        let d = drop(None, "https://example.pvt.k12.ma.us".to_string());
        assert_eq!(d.domain(), Some("example.pvt.k12.ma.us".to_string()));

        let d = drop(None, "https:///".to_string());
        assert_eq!(d.domain(), None);
    }

    fn drop(title: Option<String>, url: String) -> Drop {
        let now = chrono::Utc::now().naive_utc();
        Drop {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            title,
            url,
            status: DropStatus::Unread,
            moved_at: now,
            created_at: now,
            updated_at: now,
        }
    }
}
