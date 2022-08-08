use diesel_async::{AsyncConnection, AsyncPgConnection};
use uuid::Uuid;

use crate::models::{
    Drop as DropRecord, DropStatus, DropTag, NewDrop, NewDropTag, NewTag, Tag, User,
};
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

    db.transaction::<Vec<Drop>, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let drops: Vec<DropRecord> = d::drops
                .filter(d::user_id.eq(user.id).and(d::status.eq(status)))
                .load(conn)
                .await?;

            let drop_tags: Vec<Vec<(DropTag, Tag)>> = DropTag::belonging_to(&drops)
                .inner_join(t::tags)
                .load(conn)
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
        })
    })
    .await
}

pub async fn find_drop(db: &mut AsyncPgConnection, user: &User, id: Uuid) -> anyhow::Result<Drop> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as d;

    db.transaction::<Drop, anyhow::Error, _>(|conn| {
        let user_id = user.id;

        Box::pin(async move {
            let drop: DropRecord = d::drops
                .filter(d::user_id.eq(user_id).and(d::id.eq(id)))
                .get_result(conn)
                .await?;

            let tags = load_drop_tags(conn, &drop).await?;

            Ok(Drop { drop, tags })
        })
    })
    .await
}

async fn load_drop_tags(
    conn: &mut AsyncPgConnection,
    drop: &DropRecord,
) -> anyhow::Result<Vec<Tag>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;
    use schema::tags::dsl as t;

    let tag_ids: Vec<Uuid> = DropTag::belonging_to(&drop)
        .select(dt::tag_id)
        .load(conn)
        .await?;

    let tags: Vec<Tag> = t::tags.filter(t::id.eq_any(tag_ids)).load(conn).await?;

    Ok(tags)
}

pub async fn create_drop(
    db: &mut AsyncPgConnection,
    user: User,
    title: Option<String>,
    url: String,
    tags: Option<Vec<TagSelector>>,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as t;

    db.transaction::<Drop, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let drop: DropRecord = insert_into(t::drops)
                .values(&NewDrop {
                    user_id: user.id,
                    title: title.as_deref(),
                    url: &url,
                    status: DropStatus::Unread,
                    moved_at: now.naive_utc(),
                })
                .get_result(conn)
                .await?;

            let selectors = tags;
            let mut tags = Vec::new();
            for sel in selectors.unwrap_or_default() {
                let tag = find_or_create_tag(conn, &user, sel).await?;
                tags.push(tag);
            }

            attach_tags(conn, &drop, &tags).await?;

            Ok(Drop { drop, tags })
        })
    })
    .await
}

#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::drops)]
pub struct DropFields {
    pub title: Option<String>,
    pub url: Option<String>,
}

pub async fn update_drop(
    db: &mut AsyncPgConnection,
    user: User,
    drop: Drop,
    fields: DropFields,
    tags: Option<Vec<TagSelector>>,
) -> anyhow::Result<Drop> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    db.transaction::<Drop, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let drop: DropRecord = update(&drop.drop).set(fields).get_result(conn).await?;

            let tags = match tags {
                None => load_drop_tags(conn, &drop).await?,
                Some(selectors) => {
                    let mut tags = Vec::new();
                    for sel in selectors {
                        let tag = find_or_create_tag(conn, &user, sel).await?;
                        tags.push(tag);
                    }

                    attach_tags(conn, &drop, &tags).await?;
                    detach_other_tags(conn, &drop, &tags).await?;

                    tags
                }
            };

            Ok(Drop { drop, tags })
        })
    })
    .await
}

pub async fn move_drop(
    db: &mut AsyncPgConnection,
    drop: Drop,
    status: DropStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    use diesel::{update, ExpressionMethods};
    use diesel_async::RunQueryDsl;
    use schema::drops::dsl as d;

    db.transaction::<Drop, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let drop: DropRecord = update(&drop.drop)
                .set((d::status.eq(status), d::moved_at.eq(now.naive_utc())))
                .get_result(conn)
                .await?;

            let tags = load_drop_tags(conn, &drop).await?;

            Ok(Drop { drop, tags })
        })
    })
    .await
}

pub async fn list_tags(db: &mut AsyncPgConnection, user: &User) -> anyhow::Result<Vec<Tag>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let res: Vec<Tag> = t::tags.filter(t::user_id.eq(user.id)).load(db).await?;
    Ok(res)
}

#[derive(Debug, Clone)]
pub enum TagSelector {
    Find { id: Uuid },
    Create { name: String, color: String },
}

pub async fn find_or_create_tag(
    db: &mut AsyncPgConnection,
    user: &User,
    sel: TagSelector,
) -> anyhow::Result<Tag> {
    match sel {
        TagSelector::Find { id } => find_tag(db, user, id).await,
        TagSelector::Create { name, color } => create_tag(db, user, &name, &color).await,
    }
}

pub async fn find_tag(db: &mut AsyncPgConnection, user: &User, id: Uuid) -> anyhow::Result<Tag> {
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

pub async fn attach_tag(
    db: &mut AsyncPgConnection,
    drop: &DropRecord,
    tag: &Tag,
) -> anyhow::Result<DropTag> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;

    let dt: DropTag = insert_into(dt::drop_tags)
        .values(&NewDropTag {
            drop_id: drop.id,
            tag_id: tag.id,
        })
        .on_conflict((dt::drop_id, dt::tag_id))
        .do_nothing()
        .get_result(db)
        .await?;
    Ok(dt)
}

pub async fn attach_tags(
    db: &mut AsyncPgConnection,
    drop: &DropRecord,
    tags: &[Tag],
) -> anyhow::Result<Vec<DropTag>> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;

    let values = tags
        .iter()
        .map(|t| NewDropTag {
            drop_id: drop.id,
            tag_id: t.id,
        })
        .collect::<Vec<NewDropTag>>();

    let dts: Vec<DropTag> = insert_into(dt::drop_tags)
        .values(&values)
        .on_conflict((dt::drop_id, dt::tag_id))
        .do_nothing()
        .get_results(db)
        .await?;
    Ok(dts)
}

pub async fn detach_other_tags(
    db: &mut AsyncPgConnection,
    drop: &DropRecord,
    tags: &[Tag],
) -> anyhow::Result<()> {
    use diesel::delete;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;

    let tag_ids: Vec<Uuid> = tags.iter().map(|tag| tag.id).collect();

    delete(dt::drop_tags.filter(dt::drop_id.eq(drop.id).and(dt::tag_id.ne_all(tag_ids))))
        .execute(db)
        .await?;

    Ok(())
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
