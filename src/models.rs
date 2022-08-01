use chrono::NaiveDateTime as Timestamp;
use diesel::{
    deserialize::{self, FromSql},
    pg::{Pg, PgValue},
    serialize::{self, IsNull, Output, ToSql},
    AsExpression, FromSqlRow, Insertable, Queryable,
};
use serde::Deserialize;
use std::io::Write;
use uuid::Uuid;

use crate::{schema, sql_types};

// Remember: using `#[derive(Queryable)]` assumes that the order of fields on the `Model` struct
// matches the order of columns in the `models` table (stored in `schema.rs`).

#[derive(Debug, Clone, PartialEq, Eq, Queryable)]
pub struct User {
    pub id: Uuid,
    pub stytch_user_id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Deserialize, Insertable, Debug, Clone)]
#[diesel(table_name = schema::users)]
pub struct NewUser<'a> {
    pub stytch_user_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, FromSqlRow, AsExpression)]
#[diesel(sql_type = sql_types::Drop_status)]
#[serde(rename_all = "lowercase")]
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

impl ToSql<sql_types::Drop_status, Pg> for DropStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            DropStatus::Unread => out.write_all(b"unread")?,
            DropStatus::Read => out.write_all(b"read")?,
            DropStatus::Saved => out.write_all(b"saved")?,
        }
        Ok(IsNull::No)
    }
}

impl FromSql<sql_types::Drop_status, Pg> for DropStatus {
    fn from_sql(bytes: PgValue<'_>) -> deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"unread" => Ok(DropStatus::Unread),
            b"read" => Ok(DropStatus::Read),
            b"saved" => Ok(DropStatus::Saved),
            _ => Err("Unrecognized enum variant".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Identifiable, Associations)]
#[diesel(table_name=schema::drops)]
#[diesel(belongs_to(User))]
pub struct Drop {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: Option<String>,
    pub url: String,
    pub status: DropStatus,
    pub moved_at: Timestamp,
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

#[derive(Debug, Clone, Deserialize, Insertable, Associations)]
#[diesel(table_name = schema::drops)]
#[diesel(belongs_to(User))]
pub struct NewDrop<'a> {
    pub user_id: Uuid,
    pub title: Option<&'a str>,
    pub url: &'a str,
    pub status: DropStatus,
    pub moved_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Identifiable, Associations)]
#[diesel(table_name=schema::tags)]
#[diesel(belongs_to(User))]
pub struct Tag {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Deserialize, Insertable, Debug, Clone)]
#[diesel(table_name = schema::tags)]
pub struct NewTag<'a> {
    pub user_id: Uuid,
    pub name: &'a str,
    pub color: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Identifiable, Associations)]
#[diesel(table_name = schema::drop_tags)]
#[diesel(belongs_to(Drop))]
#[diesel(belongs_to(Tag))]
pub struct DropTag {
    pub id: Uuid,
    pub drop_id: Uuid,
    pub tag_id: Uuid,
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
