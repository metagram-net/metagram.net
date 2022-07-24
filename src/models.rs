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

#[derive(Queryable, Debug, Clone)]
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

#[derive(Queryable, Debug, Clone)]
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
    pub fn domain(&self) -> String {
        "example.com".to_string() // TODO: Do the PSL thing
    }

    pub fn display_text(&self) -> String {
        self.title.as_ref().unwrap_or(&self.url).to_string()
    }
}

#[derive(Deserialize, Insertable, Debug, Clone)]
#[diesel(table_name = schema::drops)]
pub struct NewDrop<'a> {
    pub user_id: Uuid,
    pub title: Option<&'a str>,
    pub url: &'a str,
    pub status: DropStatus,
    pub moved_at: Timestamp,
}
