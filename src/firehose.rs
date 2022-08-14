use std::collections::HashMap;

use diesel_async::{AsyncConnection, AsyncPgConnection};
use uuid::Uuid;

use crate::models::{
    Drop as DropRecord, DropTag, Hydrant as HydrantRecord, NewDrop, NewDropTag, NewHydrant,
    NewStream, NewTag, Stream as StreamRecord, User,
};
pub use crate::models::{DropStatus, Tag};
use crate::schema;

pub struct Drop {
    pub drop: DropRecord,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Default)]
pub struct DropFilters {
    pub status: Option<DropStatus>,
    pub tags: Option<Vec<Tag>>,
}

pub async fn list_drops(
    db: &mut AsyncPgConnection,
    user: User,
    filters: DropFilters,
) -> anyhow::Result<Vec<Drop>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::drop_tags::dsl as dt;
    use schema::drops::dsl as d;
    use schema::tags::dsl as t;

    db.transaction::<Vec<Drop>, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let mut query = DropRecord::belonging_to(&user)
                .left_join(dt::drop_tags.inner_join(t::tags))
                .select(d::drops::all_columns())
                .distinct()
                .order_by(d::moved_at.asc())
                .into_boxed();

            if let Some(status) = filters.status {
                query = query.filter(d::status.eq(status));
            }
            if let Some(tags) = filters.tags {
                let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();
                query = query.filter(t::id.eq_any(tag_ids));
            }

            let drops: Vec<DropRecord> = query.load(conn).await?;

            // TODO: The query above probably sees enough data to skip the rest of this.

            let drop_tags: Vec<Vec<(DropTag, Tag)>> = DropTag::belonging_to(&drops)
                .inner_join(t::tags)
                .load(conn)
                .await?
                .grouped_by(&drops);

            let data = drops
                .into_iter()
                .zip(drop_tags)
                .map(|(drop, dts)| {
                    let mut tags: Vec<Tag> = dts.iter().cloned().map(|(_dt, tag)| tag).collect();
                    tags.sort_by_key(|t| t.name.clone());
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

    let res: Vec<Tag> = Tag::belonging_to(&user)
        .order_by(t::name.asc())
        .load(db)
        .await?;
    Ok(res)
}

pub async fn find_tags(
    db: &mut AsyncPgConnection,
    user: &User,
    ids: &[Uuid],
) -> anyhow::Result<Vec<Tag>> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let query = Tag::belonging_to(&user).filter(t::id.eq_any(ids));
    Ok(query.get_results(db).await?)
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

    Ok(Tag::belonging_to(&user).find(id).get_result(db).await?)
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

#[derive(Debug, Clone)]
pub struct CustomStream {
    pub stream: StreamRecord,
    pub tags: Vec<Tag>,
}

impl CustomStream {
    pub fn tag_names(&self) -> Vec<String> {
        self.tags.iter().cloned().map(|t| t.name).collect()
    }

    pub fn filters(&self) -> DropFilters {
        DropFilters {
            tags: Some(self.tags.to_vec()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusStream {
    pub status: DropStatus,
}

impl StatusStream {
    pub fn filters(&self) -> DropFilters {
        DropFilters {
            status: Some(self.status.clone()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stream {
    Custom(CustomStream),
    Status(StatusStream),
}

impl Stream {
    pub fn filters(&self) -> DropFilters {
        match self {
            Self::Custom(stream) => stream.filters(),
            Self::Status(stream) => stream.filters(),
        }
    }
}

pub fn status_streams() -> Vec<Stream> {
    let statuses = vec![DropStatus::Unread, DropStatus::Read, DropStatus::Saved];
    statuses
        .iter()
        .cloned()
        .map(|status| Stream::Status(StatusStream { status }))
        .collect()
}

pub async fn list_streams(db: &mut AsyncPgConnection, user: &User) -> anyhow::Result<Vec<Stream>> {
    use diesel::dsl::array;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::streams::dsl as s;
    use schema::tags::dsl as t;

    let streams: Vec<StreamRecord> = StreamRecord::belonging_to(&user)
        .order_by(s::name.asc())
        .load(db)
        .await?;

    let stream_ids: Vec<Uuid> = streams.iter().cloned().map(|s| s.id).collect();

    let tags: Vec<(Tag, StreamRecord)> = Tag::belonging_to(&user)
        .inner_join(s::streams.on(s::tag_ids.contains(array((t::id,)))))
        .filter(s::id.eq_any(&stream_ids))
        .get_results(db)
        .await?;

    let tag_sets: Vec<Vec<Tag>> = {
        let mut map: HashMap<Uuid, Vec<Tag>> = HashMap::new();

        for (tag, stream) in tags {
            map.entry(stream.id).or_insert_with(Vec::new).push(tag);
        }

        let mut out: Vec<Vec<Tag>> = Vec::new();
        for id in stream_ids {
            out.push(map.remove(&id).unwrap_or_default());
        }
        out
    };

    let mut custom_streams: Vec<Stream> = streams
        .into_iter()
        .zip(tag_sets)
        .map(|(stream, tags)| Stream::Custom(CustomStream { stream, tags }))
        .collect();

    let mut res = status_streams();
    res.append(&mut custom_streams);
    Ok(res)
}

pub async fn find_stream(
    db: &mut AsyncPgConnection,
    user: &User,
    id: Uuid,
) -> anyhow::Result<CustomStream> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let stream: StreamRecord = StreamRecord::belonging_to(&user)
        .find(id)
        .get_result(db)
        .await?;

    let tags: Vec<Tag> = Tag::belonging_to(&user)
        .filter(t::id.eq_any(&stream.tag_ids))
        .get_results(db)
        .await?;

    Ok(CustomStream { stream, tags })
}

pub async fn create_stream(
    db: &mut AsyncPgConnection,
    user: &User,
    name: &str,
    tags: &[Tag],
) -> anyhow::Result<CustomStream> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::streams::dsl as s;

    let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();
    let user = user.clone();
    let name = name.to_string();

    db.transaction::<CustomStream, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let stream: StreamRecord = insert_into(s::streams)
                .values(&NewStream {
                    user_id: user.id,
                    name: &name,
                    tag_ids,
                })
                .get_result(conn)
                .await?;

            let tags = find_tags(conn, &user, &stream.tag_ids).await?;

            Ok(CustomStream { stream, tags })
        })
    })
    .await
}

#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::streams)]
pub struct StreamFields {
    pub name: Option<String>,
    pub tag_ids: Option<Vec<Uuid>>,
}

pub async fn update_stream(
    db: &mut AsyncPgConnection,
    user: &User,
    stream: &StreamRecord,
    fields: StreamFields,
) -> anyhow::Result<CustomStream> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let stream: StreamRecord = update(stream).set(fields).get_result(db).await?;

    let tags = find_tags(db, user, &stream.tag_ids).await?;

    Ok(CustomStream { stream, tags })
}

#[derive(Debug, Clone)]
pub struct Hydrant {
    pub hydrant: HydrantRecord,
    pub tags: Vec<Tag>,
}

pub async fn list_hydrants(
    db: &mut AsyncPgConnection,
    user: &User,
) -> anyhow::Result<Vec<Hydrant>> {
    use diesel::dsl::array;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::hydrants::dsl as h;
    use schema::tags::dsl as t;

    let hydrants: Vec<HydrantRecord> = HydrantRecord::belonging_to(&user)
        .order_by(h::name.asc())
        .load(db)
        .await?;

    let hydrant_ids: Vec<Uuid> = hydrants.iter().cloned().map(|s| s.id).collect();

    let tags: Vec<(Tag, HydrantRecord)> = Tag::belonging_to(&user)
        .inner_join(h::hydrants.on(h::tag_ids.contains(array((t::id,)))))
        .filter(h::id.eq_any(&hydrant_ids))
        .get_results(db)
        .await?;

    let tag_sets: Vec<Vec<Tag>> = {
        let mut map: HashMap<Uuid, Vec<Tag>> = HashMap::new();

        for (tag, hydrant) in tags {
            map.entry(hydrant.id).or_insert_with(Vec::new).push(tag);
        }

        let mut out: Vec<Vec<Tag>> = Vec::new();
        for id in hydrant_ids {
            out.push(map.remove(&id).unwrap_or_default());
        }
        out
    };

    let res: Vec<Hydrant> = hydrants
        .into_iter()
        .zip(tag_sets)
        .map(|(hydrant, tags)| Hydrant { hydrant, tags })
        .collect();
    Ok(res)
}

pub async fn find_hydrant(
    db: &mut AsyncPgConnection,
    user: &User,
    id: Uuid,
) -> anyhow::Result<Hydrant> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use schema::tags::dsl as t;

    let hydrant: HydrantRecord = HydrantRecord::belonging_to(&user)
        .find(id)
        .get_result(db)
        .await?;

    let tags: Vec<Tag> = Tag::belonging_to(&user)
        .filter(t::id.eq_any(&hydrant.tag_ids))
        .get_results(db)
        .await?;

    Ok(Hydrant { hydrant, tags })
}

pub async fn create_hydrant(
    db: &mut AsyncPgConnection,
    user: &User,
    name: &str,
    url: &str,
    active: bool,
    tags: &[Tag],
) -> anyhow::Result<Hydrant> {
    use diesel::insert_into;
    use diesel_async::RunQueryDsl;
    use schema::hydrants::dsl as h;

    let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();
    let user = user.clone();
    let name = name.to_string();
    let url = url.to_string();

    db.transaction::<Hydrant, anyhow::Error, _>(|conn| {
        Box::pin(async move {
            let hydrant: HydrantRecord = insert_into(h::hydrants)
                .values(&NewHydrant {
                    user_id: user.id,
                    name: &name,
                    url: &url,
                    active,
                    tag_ids,
                })
                .get_result(conn)
                .await?;

            let tags = find_tags(conn, &user, &hydrant.tag_ids).await?;

            Ok(Hydrant { hydrant, tags })
        })
    })
    .await
}

// TODO: Move *Fields to models?
#[derive(Default, AsChangeset)]
#[diesel(table_name=schema::hydrants)]
pub struct HydrantFields {
    pub name: Option<String>,
    pub url: Option<String>,
    pub active: Option<bool>,
    pub tag_ids: Option<Vec<Uuid>>,
}

pub async fn update_hydrant(
    db: &mut AsyncPgConnection,
    user: &User,
    hydrant: &HydrantRecord,
    fields: HydrantFields,
) -> anyhow::Result<Hydrant> {
    use diesel::update;
    use diesel_async::RunQueryDsl;

    let hydrant: HydrantRecord = update(hydrant).set(fields).get_result(db).await?;

    let tags = find_tags(db, user, &hydrant.tag_ids).await?;

    Ok(Hydrant { hydrant, tags })
}
