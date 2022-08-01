use diesel_async::AsyncPgConnection;
use uuid::Uuid;

use crate::models::{Drop as DropRecord, DropStatus, DropTag, NewDrop, NewTag, Tag, User};
use crate::schema;

pub struct Drop {
    pub drop: DropRecord,
    pub tags: Vec<Tag>,
}

pub async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    status: DropStatus,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as d;
    use schema::tags::dsl as t;

    let drops: Vec<DropRecord> = d::drops
        .filter(d::user_id.eq(user.id).and(d::status.eq(status)))
        .load(db)
        .await?;

    let drop_tags: Vec<Vec<(DropTag, Tag)>> = DropTag::belonging_to(&drops)
        .inner_join(t::tags)
        .load(db)
        .await?
        .grouped_by(&drops);

    let data = drops
        .into_iter()
        .zip(drop_tags)
        .map(|(drop, dts)| {
            let tags = dts.iter().cloned().map(|(_dt, tag)| tag).collect();
            Drop { drop, tags }
        })
        .collect::<Vec<_>>();

    Ok(data)
}

pub async fn find_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    id: uuid::Uuid,
) -> anyhow::Result<Drop> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;
    use schema::drops::dsl as d;
    use schema::tags::dsl as t;

    let drop: DropRecord = d::drops
        .filter(d::user_id.eq(user.id).and(d::id.eq(id)))
        .get_result(db)
        .await?;

    let tag_ids: Vec<Uuid> = DropTag::belonging_to(&drop)
        .select(dt::tag_id)
        .load(db)
        .await?;

    let tags: Vec<Tag> = t::tags.filter(t::id.eq_any(tag_ids)).load(db).await?;

    Ok(Drop { drop, tags })
}

pub async fn create_drop(
    db: &mut AsyncPgConnection,
    user: &User,
    title: Option<String>,
    url: String,
    now: chrono::DateTime<chrono::Utc>,
    // TODO(tags): set tags
) -> anyhow::Result<Drop> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    let drop: DropRecord = insert_into(t::drops)
        .values(&NewDrop {
            user_id: user.id,
            title: title.as_deref(),
            url: &url,
            status: DropStatus::Unread,
            moved_at: now.naive_utc(),
        })
        .get_result(db)
        .await?;

    Ok(Drop { drop, tags: vec![] }) // TODO(tags): fetch tags
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
    // TODO(tags): set tags
) -> anyhow::Result<Drop> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let drop: DropRecord = update(&drop.drop).set(fields).get_result(db).await?;
    Ok(Drop { drop, tags: vec![] }) // TODO(tags): fetch tags
}

pub async fn move_drop(
    db: &mut AsyncPgConnection,
    drop: &Drop,
    status: DropStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::prelude::*;
    use diesel::{update, ExpressionMethods};
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;
    use schema::drops::dsl as d;
    use schema::tags::dsl as t;

    let drop: DropRecord = update(&drop.drop)
        .set((d::status.eq(status), d::moved_at.eq(now.naive_utc())))
        .get_result(db)
        .await?;

    let tag_ids: Vec<Uuid> = DropTag::belonging_to(&drop)
        .select(dt::tag_id)
        .load(db)
        .await?;

    let tags: Vec<Tag> = t::tags.filter(t::id.eq_any(tag_ids)).load(db).await?;

    Ok(Drop { drop, tags })
}

pub async fn list_tags(db: &mut AsyncPgConnection, user: &User) -> anyhow::Result<Vec<Tag>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let res: Vec<Tag> = t::tags.filter(t::user_id.eq(user.id)).load(db).await?;
    Ok(res)
}

pub async fn find_tag(
    db: &mut AsyncPgConnection,
    user: &User,
    id: uuid::Uuid,
) -> anyhow::Result<Tag> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let res: Tag = t::tags
        .filter(t::user_id.eq(user.id).and(t::id.eq(id)))
        .get_result(db)
        .await?;
    Ok(res)
}

pub async fn create_tag(
    db: &mut AsyncPgConnection,
    user: &User,
    name: &str,
    color: &str,
) -> anyhow::Result<Tag> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let tag: Tag = insert_into(t::tags)
        .values(&NewTag {
            user_id: user.id,
            name,
            color,
        })
        .get_result(db)
        .await?;
    Ok(tag)
}

#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::tags)]
pub struct TagFields {
    pub name: Option<String>,
    pub color: Option<String>,
}

pub async fn update_tag(
    db: &mut AsyncPgConnection,
    tag: &Tag,
    fields: TagFields,
) -> anyhow::Result<Tag> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let tag: Tag = update(tag).set(fields).get_result(db).await?;
    Ok(tag)
}
