use diesel_async::AsyncPgConnection;

use crate::models::{Drop, DropStatus, NewDrop, User};
use crate::schema;

pub async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    status: DropStatus,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::status.eq(status)))
        .load(db)
        .await?;
    Ok(res)
}

pub async fn find_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    id: uuid::Uuid,
) -> anyhow::Result<Drop> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let res = t::drops
        .filter(t::user_id.eq(user.id).and(t::id.eq(id)))
        .get_result(db)
        .await?;
    Ok(res)
}

pub async fn create_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    title: Option<String>,
    url: String,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: Drop = insert_into(t::drops)
        .values(&NewDrop {
            user_id: user.id,
            title: title.as_deref(),
            url: &url,
            status: DropStatus::Unread,
            moved_at: now.naive_utc(),
        })
        .get_result(db)
        .await?;
    Ok(drop)
}

#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::drops)]
pub struct DropFields {
    pub title: Option<String>,
    pub url: Option<String>,
}

pub async fn update_drop(
    db: &mut AsyncPgConnection,
    drop: &Drop,
    fields: DropFields,
) -> anyhow::Result<Drop> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let drop: Drop = update(drop).set(fields).get_result(db).await?;
    Ok(drop)
}

pub async fn move_drop(
    db: &mut AsyncPgConnection,
    drop: &Drop,
    status: DropStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::{update, ExpressionMethods};
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: Drop = update(drop)
        .set((t::status.eq(status), t::moved_at.eq(now.naive_utc())))
        .get_result(db)
        .await?;
    Ok(drop)
}
