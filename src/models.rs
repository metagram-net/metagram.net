use chrono::NaiveDateTime as Timestamp;
use diesel::Queryable;
use uuid::Uuid;

// Remember: using `#[derive(Queryable)]` assumes that the order of fields on the `Model` struct
// matches the order of columns in the `models` table (stored in `schema.rs`).

#[derive(Queryable, Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub stytch_user_id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
