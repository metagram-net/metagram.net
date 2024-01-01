use std::collections::{hash_map::Entry, HashMap};

use sqlx::{Connection, PgConnection, PgExecutor, QueryBuilder};
use uuid::Uuid;

use crate::models;
pub use crate::models::{DropStatus, Tag};

type PgQueryBuilder<'a> = QueryBuilder<'a, sqlx::Postgres>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drop {
    pub drop: models::Drop,
    pub tags: Vec<models::Tag>,
}

// TODO: JoinAsBsRow + from_rows_one + from_rows_vec might generalize

impl Drop {
    fn from_row(row: JoinDropsTagsRow) -> Self {
        Self {
            drop: row.drop(),
            tags: row.tag().into_iter().collect(),
        }
    }

    fn from_rows_one(rows: Vec<JoinDropsTagsRow>) -> Self {
        let v = Self::from_rows_vec(rows);
        v[0].clone()
    }

    fn from_rows_vec(rows: Vec<JoinDropsTagsRow>) -> Vec<Self> {
        let mut drop_ids: Vec<Uuid> = Vec::new();
        let mut drop_builds: HashMap<Uuid, Self> = HashMap::new();

        for row in rows {
            let drop_id = row.drop_id;

            match drop_builds.entry(drop_id) {
                Entry::Occupied(mut entry) => {
                    if let Some(tag) = row.tag() {
                        entry.get_mut().tags.push(tag);
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(Self::from_row(row));
                    drop_ids.push(drop_id);
                }
            }
        }

        let mut data: Vec<Self> = Vec::with_capacity(drop_ids.len());
        for id in drop_ids.drain(0..) {
            let drop = drop_builds.remove(&id).expect("id must be in map");
            data.push(drop);
        }
        data
    }
}

#[derive(Debug, Clone, Default)]
pub struct DropFilters {
    pub status: Option<DropStatus>,
    pub tags: Option<Vec<models::Tag>>,
}

type Timestamp = chrono::NaiveDateTime;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct JoinDropsTagsRow {
    drop_id: Uuid,
    drop_user_id: Uuid,
    drop_title: Option<String>,
    drop_url: String,
    drop_status: DropStatus,
    drop_moved_at: Timestamp,
    drop_hydrant_id: Option<Uuid>,
    drop_created_at: Timestamp,
    drop_updated_at: Timestamp,

    tag_id: Option<Uuid>,
    tag_user_id: Option<Uuid>,
    tag_name: Option<String>,
    tag_color: Option<String>,
    tag_created_at: Option<Timestamp>,
    tag_updated_at: Option<Timestamp>,
}

impl JoinDropsTagsRow {
    fn select() -> PgQueryBuilder<'static> {
        QueryBuilder::new(
            "
            select

              drops.id         as drop_id
            , drops.user_id    as drop_user_id
            , drops.title      as drop_title
            , drops.url        as drop_url
            , drops.status     as drop_status
            , drops.moved_at   as drop_moved_at
            , drops.hydrant_id as drop_hydrant_id
            , drops.created_at as drop_created_at
            , drops.updated_at as drop_updated_at

            , tags.id         as tag_id
            , tags.user_id    as tag_user_id
            , tags.name       as tag_name
            , tags.color      as tag_color
            , tags.created_at as tag_created_at
            , tags.updated_at as tag_updated_at

            from drops
            left join drop_tags on drop_tags.drop_id = drops.id
            left join tags on tags.id = drop_tags.tag_id
            ",
        )
    }

    fn drop(&self) -> models::Drop {
        models::Drop {
            id: self.drop_id,
            user_id: self.drop_user_id,
            title: self.drop_title.clone(),
            url: self.drop_url.clone(),
            status: self.drop_status,
            moved_at: self.drop_moved_at,
            hydrant_id: self.drop_hydrant_id,
            created_at: self.drop_created_at,
            updated_at: self.drop_updated_at,
        }
    }

    fn tag(&self) -> Option<models::Tag> {
        self.tag_id?;

        Some(models::Tag {
            id: self.tag_id.unwrap(),
            user_id: self.tag_user_id.unwrap(),
            name: self.tag_name.as_ref().unwrap().clone(),
            color: self.tag_color.as_ref().unwrap().clone(),
            created_at: self.tag_created_at.unwrap(),
            updated_at: self.tag_updated_at.unwrap(),
        })
    }
}

pub async fn list_drops(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    filters: DropFilters,
    limit: Option<i64>,
) -> anyhow::Result<Vec<Drop>> {
    let mut query = QueryBuilder::new(
        "
        with drop_ids as (
          select distinct(drops.id)
          from drops
          left join drop_tags on drop_tags.drop_id = drops.id
          left join tags on tags.id = drop_tags.tag_id
        ",
    );
    query.push(" where drops.user_id = ");
    query.push_bind(user.id);

    if let Some(status) = filters.status {
        query.push(" and drops.status = ");
        query.push("CAST( ");
        query.push_bind(status.to_string());
        query.push(" as drop_status) ");
    }
    if let Some(tags) = filters.tags {
        let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();

        query.push(" and tags.id = ANY(");
        query.push_bind(tag_ids);
        query.push(")");
    }
    if let Some(limit) = limit {
        query.push("limit ");
        query.push_bind(limit);
    }
    query.push(") "); // with

    query.push(
        "
        select
            drops.id as drop_id
          , drops.user_id as drop_user_id
          , drops.title as drop_title
          , drops.url as drop_url
          , drops.status as drop_status
          , drops.moved_at as drop_moved_at
          , drops.hydrant_id as drop_hydrant_id
          , drops.created_at as drop_created_at
          , drops.updated_at as drop_updated_at
          , tags.id as tag_id
          , tags.user_id as tag_user_id
          , tags.name as tag_name
          , tags.color as tag_color
          , tags.created_at as tag_created_at
          , tags.updated_at as tag_updated_at
        from
          drops
          left join drop_tags on drop_tags.drop_id = drops.id
          left join tags on tags.id = drop_tags.tag_id
        where drops.id in (select id from drop_ids)
        order by
            drops.moved_at asc
          , drops.id asc
          , tags.name asc
        ",
    );

    let rows: Vec<JoinDropsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(Drop::from_rows_vec(rows))
}

pub async fn find_drop(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    id: Uuid,
) -> anyhow::Result<Drop> {
    let mut query = JoinDropsTagsRow::select();
    query.push(" where drops.user_id = ");
    query.push_bind(user.id);
    query.push(" and drops.id = ");
    query.push_bind(id);
    query.push(
        "
        order by tags.name asc
        ",
    );

    let rows: Vec<JoinDropsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(Drop::from_rows_one(rows))
}

async fn find_drop_record(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    id: Uuid,
) -> sqlx::Result<models::Drop> {
    sqlx::query_as(
        "
        select * from drops
        where id = $1
        and user_id = $2
        ",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(conn)
    .await
}

async fn load_drop_tags(
    conn: impl PgExecutor<'_>,
    drop: &models::Drop,
) -> sqlx::Result<Vec<models::Tag>> {
    sqlx::query_as!(
        models::Tag,
        "
        select tags.*
        from tags
        join drop_tags on drop_tags.tag_id = tags.id
        where drop_tags.drop_id = $1
        order by tags.name asc
        ",
        drop.id,
    )
    .fetch_all(conn)
    .await
}

// TODO: This function signature is _awful_. Fix it.
pub async fn create_drop(
    conn: &mut PgConnection,
    user: &models::User,
    title: Option<String>,
    url: String,
    hydrant_id: Option<Uuid>,
    tags: Option<Vec<TagSelector>>,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Drop> {
    let user = user.clone();

    conn.transaction(|tx| {
        Box::pin(async move {
            let drop: models::Drop = sqlx::query_as(
                "
                insert into drops
                (user_id, title, url, status, moved_at, hydrant_id)
                values
                ($1, $2, $3, $4::drop_status, $5, $6)
                returning *
                ",
            )
            .bind(user.id)
            .bind(title)
            .bind(url)
            .bind(DropStatus::Unread)
            .bind(now.naive_utc())
            .bind(hydrant_id)
            .fetch_one(&mut *tx)
            .await?;

            let selectors = tags;
            let mut tags = Vec::new();
            for sel in selectors.unwrap_or_default() {
                let tag = find_or_create_tag(&mut *tx, &user, sel).await?;
                tags.push(tag);
            }

            attach_tags(&mut *tx, &drop, &tags).await?;
            tags.sort_by_key(|t| t.name.clone());

            Ok(Drop { drop, tags })
        })
    })
    .await
}

#[derive(Default)]
pub struct DropFields {
    pub title: Option<String>,
    pub url: Option<String>,
}

pub async fn update_drop(
    conn: &mut PgConnection,
    user: &models::User,
    drop: &models::Drop,
    fields: DropFields,
    tags: Option<Vec<TagSelector>>,
) -> anyhow::Result<Drop> {
    let user = user.clone();
    let drop_id = drop.id;

    let mut query = QueryBuilder::new("update drops set");

    let mut assign = query.separated(" , ");
    let mut do_assign = false;
    if let Some(title) = fields.title {
        assign.push(" title = ");
        assign.push_bind_unseparated(title);
        do_assign = true;
    }
    if let Some(url) = fields.url {
        assign.push(" url = ");
        assign.push_bind_unseparated(url);
        do_assign = true;
    }

    query.push(" where id = ");
    query.push_bind(drop_id);
    query.push(" and user_id = ");
    query.push_bind(user.id);
    query.push(" returning *");

    conn.transaction(|tx| {
        Box::pin(async move {
            let drop = if do_assign {
                query.build_query_as().fetch_one(&mut *tx).await?
            } else {
                find_drop_record(&mut *tx, &user, drop_id).await?
            };

            let tags = if let Some(selectors) = tags {
                let mut tags = Vec::new();
                for sel in selectors {
                    let tag = find_or_create_tag(&mut *tx, &user, sel).await?;
                    tags.push(tag);
                }

                attach_tags(&mut *tx, &drop, &tags).await?;
                detach_other_tags(&mut *tx, &drop, &tags).await?;

                tags
            } else {
                load_drop_tags(&mut *tx, &drop).await?
            };

            Ok(Drop { drop, tags })
        })
    })
    .await
}

pub async fn move_drop(
    conn: &mut PgConnection,
    drop: Drop,
    status: DropStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Drop> {
    let query = sqlx::query_as(
        "
        update drops
        set status = $2::drop_status, moved_at = $3
        where id = $1
        returning *
        ",
    )
    .bind(drop.drop.id)
    .bind(status)
    .bind(now.naive_utc());

    conn.transaction(|tx| {
        Box::pin(async move {
            let drop = query.fetch_one(&mut *tx).await?;
            let tags = load_drop_tags(&mut *tx, &drop).await?;
            Ok(Drop { drop, tags })
        })
    })
    .await
}

pub async fn list_tags(
    conn: impl PgExecutor<'_>,
    user: &models::User,
) -> anyhow::Result<Vec<models::Tag>> {
    let tags = sqlx::query_as!(
        models::Tag,
        "
        select * from tags
        where user_id = $1
        order by name asc
        ",
        user.id,
    )
    .fetch_all(conn)
    .await?;
    Ok(tags)
}

pub async fn find_tags(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    ids: &[Uuid],
) -> sqlx::Result<Vec<models::Tag>> {
    let tags = sqlx::query_as!(
        models::Tag,
        "
        select * from tags
        where user_id = $1 and id = ANY($2)
        order by name asc
        ",
        user.id,
        ids,
    )
    .fetch_all(conn)
    .await?;
    Ok(tags)
}

#[derive(Debug, Clone)]
pub enum TagSelector {
    Find { id: Uuid },
    Create { name: String, color: String },
}

pub async fn find_or_create_tag(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    sel: TagSelector,
) -> sqlx::Result<models::Tag> {
    match sel {
        TagSelector::Find { id } => find_tag(conn, user, id).await,
        TagSelector::Create { name, color } => create_tag(conn, user, &name, &color).await,
    }
}

pub async fn find_tag(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    id: Uuid,
) -> sqlx::Result<models::Tag> {
    sqlx::query_as!(
        models::Tag,
        "
        select * from tags
        where user_id = $1 and id = $2
        ",
        user.id,
        id,
    )
    .fetch_one(conn)
    .await
}

pub async fn create_tag(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    name: &str,
    color: &str,
) -> sqlx::Result<models::Tag> {
    sqlx::query_as!(
        models::Tag,
        "
        insert into tags (user_id, name, color)
        values ($1, $2, $3)
        returning *
        ",
        user.id,
        name,
        color,
    )
    .fetch_one(conn)
    .await
}

pub async fn attach_tags(
    conn: impl PgExecutor<'_>,
    drop: &models::Drop,
    tags: &[models::Tag],
) -> sqlx::Result<Vec<models::DropTag>> {
    if tags.is_empty() {
        // This query would be a syntax error anyway, so skip it.
        return Ok(vec![]);
    }

    let mut query = QueryBuilder::new(
        "
        insert into drop_tags
        (drop_id, tag_id)
        ",
    );

    query.push_values(tags.iter(), |mut q, tag| {
        q.push_bind(drop.id).push_bind(tag.id);
    });

    query.push(" on conflict do nothing ");
    query.push(" returning *");

    query.build_query_as().fetch_all(conn).await
}

pub async fn detach_other_tags(
    conn: impl PgExecutor<'_>,
    drop: &models::Drop,
    keep_tags: &[models::Tag],
) -> sqlx::Result<Vec<models::DropTag>> {
    if keep_tags.is_empty() {
        // This query would be a syntax error anyway, so skip it.
        return Ok(vec![]);
    }

    let tag_ids: Vec<Uuid> = keep_tags.iter().map(|tag| tag.id).collect();
    sqlx::query_as!(
        models::DropTag,
        "
        delete from drop_tags
        where drop_id = $1
        and not tag_id = ANY($2)
        returning *
        ",
        drop.id,
        &tag_ids,
    )
    .fetch_all(conn)
    .await
}

#[derive(Default)]
pub struct TagFields {
    pub name: Option<String>,
    pub color: Option<String>,
}

pub async fn update_tag(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    tag: models::Tag,
    fields: TagFields,
) -> sqlx::Result<models::Tag> {
    let mut query = QueryBuilder::new("update tags set");

    let mut assign = query.separated(" , ");
    let mut do_assign = false;
    if let Some(name) = fields.name {
        assign.push(" name = ");
        assign.push_bind_unseparated(name);
        do_assign = true;
    }
    if let Some(color) = fields.color {
        assign.push(" color = ");
        assign.push_bind_unseparated(color);
        do_assign = true;
    }

    query.push(" where id = ");
    query.push_bind(tag.id);
    query.push(" and user_id = ");
    query.push_bind(user.id);
    query.push(" returning *");

    if do_assign {
        query.build_query_as().fetch_one(conn).await
    } else {
        find_tag(conn, user, tag.id).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomStream {
    pub stream: models::Stream,
    pub tags: Vec<models::Tag>,
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StatusStream {
    pub status: DropStatus,
}

impl StatusStream {
    pub fn new(status: DropStatus) -> Self {
        Self { status }
    }

    pub fn filters(&self) -> DropFilters {
        DropFilters {
            status: Some(self.status),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub fn status_streams() -> Vec<StatusStream> {
    let statuses = [DropStatus::Unread, DropStatus::Read, DropStatus::Saved];
    statuses.into_iter().map(StatusStream::new).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct JoinStreamsTagsRow {
    stream_id: Uuid,
    stream_user_id: Uuid,
    stream_name: String,
    stream_tag_ids: Vec<Uuid>,
    stream_created_at: Timestamp,
    stream_updated_at: Timestamp,

    tag_id: Uuid,
    tag_user_id: Uuid,
    tag_name: String,
    tag_color: String,
    tag_created_at: Timestamp,
    tag_updated_at: Timestamp,
}

impl JoinStreamsTagsRow {
    fn select() -> PgQueryBuilder<'static> {
        QueryBuilder::new(
            "
            select

              streams.id         as stream_id
            , streams.user_id    as stream_user_id
            , streams.name       as stream_name
            , streams.tag_ids    as stream_tag_ids
            , streams.created_at as stream_created_at
            , streams.updated_at as stream_updated_at

            , tags.id         as tag_id
            , tags.user_id    as tag_user_id
            , tags.name       as tag_name
            , tags.color      as tag_color
            , tags.created_at as tag_created_at
            , tags.updated_at as tag_updated_at

            from streams
            join tags on tags.id = ANY(streams.tag_ids)
            ",
        )
    }

    fn stream(&self) -> models::Stream {
        models::Stream {
            id: self.stream_id,
            user_id: self.stream_user_id,
            name: self.stream_name.clone(),
            tag_ids: self.stream_tag_ids.clone(),
            created_at: self.stream_created_at,
            updated_at: self.stream_updated_at,
        }
    }

    fn tag(&self) -> models::Tag {
        models::Tag {
            id: self.tag_id,
            user_id: self.tag_user_id,
            name: self.tag_name.clone(),
            color: self.tag_color.clone(),
            created_at: self.tag_created_at,
            updated_at: self.tag_updated_at,
        }
    }
}

impl CustomStream {
    fn from_row(row: JoinStreamsTagsRow) -> Self {
        Self {
            stream: row.stream(),
            tags: vec![row.tag()],
        }
    }

    fn from_rows_one(rows: Vec<JoinStreamsTagsRow>) -> Self {
        let v = Self::from_rows_vec(rows);
        v[0].clone()
    }

    fn from_rows_vec(rows: Vec<JoinStreamsTagsRow>) -> Vec<Self> {
        let mut stream_ids: Vec<Uuid> = Vec::new();
        let mut stream_builds: HashMap<Uuid, Self> = HashMap::new();

        for row in rows {
            let stream_id = row.stream_id;

            match stream_builds.entry(stream_id) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().tags.push(row.tag());
                }
                Entry::Vacant(entry) => {
                    entry.insert(Self::from_row(row));
                    stream_ids.push(stream_id);
                }
            }
        }

        let mut data: Vec<Self> = Vec::with_capacity(stream_ids.len());
        for id in stream_ids.drain(0..) {
            let stream = stream_builds.remove(&id).expect("id must be in map");
            data.push(stream);
        }
        data
    }
}

pub async fn custom_streams(
    conn: impl PgExecutor<'_>,
    user: &models::User,
) -> sqlx::Result<Vec<CustomStream>> {
    let user = user.clone();

    let mut query = JoinStreamsTagsRow::select();
    query.push(" where streams.user_id = ");
    query.push_bind(user.id);
    query.push(
        "
        order by
            streams.name asc
          , streams.created_at asc
          , tags.name asc
        ",
    );

    let rows: Vec<JoinStreamsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(CustomStream::from_rows_vec(rows))
}

pub async fn list_streams(
    conn: impl PgExecutor<'_>,
    user: &models::User,
) -> anyhow::Result<Vec<Stream>> {
    let mut common = status_streams();
    let mut custom = custom_streams(conn, user).await?;

    let mut all = Vec::with_capacity(common.len() + custom.len());
    for stream in common.drain(0..) {
        all.push(Stream::Status(stream))
    }
    for stream in custom.drain(0..) {
        all.push(Stream::Custom(stream))
    }
    Ok(all)
}

pub async fn find_stream(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    id: Uuid,
) -> sqlx::Result<CustomStream> {
    let user = user.clone();

    let mut query = JoinStreamsTagsRow::select();
    query.push(" where streams.user_id = ");
    query.push_bind(user.id);
    query.push(" and streams.id = ");
    query.push_bind(id);
    query.push(
        "
        order by
            streams.name asc
          , streams.created_at asc
          , tags.name asc
        ",
    );

    let rows: Vec<JoinStreamsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(CustomStream::from_rows_one(rows))
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Stream(#[from] StreamError),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StreamError {
    #[error("no tags specified")]
    NoTags,
}

pub async fn create_stream(
    conn: &mut PgConnection,
    user: &models::User,
    name: &str,
    tags: &[models::Tag],
) -> Result<CustomStream, Error> {
    if tags.is_empty() {
        return Err(StreamError::NoTags)?;
    }

    let user = user.clone();
    let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();

    let query = sqlx::query_as!(
        models::Stream,
        "
        insert into streams
        (user_id, name, tag_ids)
        values
        ($1, $2, $3)
        returning *
        ",
        user.id,
        name,
        &tag_ids,
    );

    conn.transaction(|tx| {
        Box::pin(async move {
            let stream = query.fetch_one(&mut *tx).await?;
            let tags = find_tags(&mut *tx, &user, &stream.tag_ids).await?;
            Ok(CustomStream { stream, tags })
        })
    })
    .await
}

#[derive(Default)]
pub struct StreamFields {
    pub name: Option<String>,
    pub tag_ids: Option<Vec<Uuid>>,
}

pub async fn update_stream(
    conn: &mut PgConnection,
    user: &models::User,
    stream: &models::Stream,
    fields: StreamFields,
) -> sqlx::Result<CustomStream> {
    let user = user.clone();
    let stream_id = stream.id;

    let mut query = QueryBuilder::new("update streams set");

    let mut assign = query.separated(" , ");
    let mut do_assign = false;
    if let Some(name) = fields.name {
        assign.push(" name = ");
        assign.push_bind_unseparated(name);
        do_assign = true;
    }
    if let Some(tag_ids) = fields.tag_ids {
        assign.push(" tag_ids = ");
        assign.push_bind_unseparated(tag_ids);
        do_assign = true;
    }

    query.push(" where id = ");
    query.push_bind(stream.id);
    query.push(" and user_id = ");
    query.push_bind(user.id);
    query.push(" returning *");

    conn.transaction(|tx| {
        Box::pin(async move {
            if do_assign {
                let stream: models::Stream = query.build_query_as().fetch_one(&mut *tx).await?;
                let tags = find_tags(&mut *tx, &user, &stream.tag_ids).await?;

                Ok(CustomStream { stream, tags })
            } else {
                find_stream(&mut *tx, &user, stream_id).await
            }
        })
    })
    .await
}

struct Story {
    title: Option<String>,
    url: String,
}

fn extract_stories(
    channel: rss::Channel,
    now: chrono::DateTime<chrono::Utc>,
    last_fetched: Option<Timestamp>,
) -> Vec<Story> {
    channel
        .items
        .into_iter()
        .filter_map(|item| {
            let title = item.title;

            // The `link` field is optional in RSS, but Firehose doesn't make sense without one.
            let url = item.link?;
            // All dates are optional, so assume that anything without a publish date is new. The
            // URL itself is our last chance to de-dupe, and if that doesn't catch it, maybe it's
            // truly new content.  ¯\_(ツ)_/¯
            let published_at = item
                .pub_date
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or(now);

            if let Some(fetched_at) = last_fetched {
                if published_at.naive_utc() < fetched_at {
                    // We've (probably) already imported this item.
                    return None;
                }
            }

            Some(Story { title, url })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hydrant {
    pub hydrant: models::Hydrant,
    pub tags: Vec<models::Tag>,
}

impl Hydrant {
    pub async fn fetch(
        conn: &mut PgConnection,
        client: &reqwest::Client,
        id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        let mut tx = conn.begin().await?;

        // Take a lock on the row to prevent parallel fetches.
        //
        // TODO: Could this be a `for no key update`?
        let hydrant = sqlx::query_as!(
            models::Hydrant,
            "
            select *
            from hydrants
            where id = $1
            for update
            ",
            id,
        )
        .fetch_one(&mut tx)
        .await?;

        // Ignore inactive hydrants.
        if !hydrant.active {
            return Ok(());
        }

        let content = client
            .request(http::Method::GET, &hydrant.url)
            .send()
            .await?
            .bytes()
            .await?;
        let channel = rss::Channel::read_from(&content[..])?;

        let user = crate::auth::find_user(&mut tx, hydrant.user_id).await?;

        let stories = extract_stories(channel, now, hydrant.fetched_at);

        let tag_selectors: Vec<TagSelector> = hydrant
            .tag_ids
            .iter()
            .cloned()
            .map(|id| TagSelector::Find { id })
            .collect();

        for story in stories {
            create_drop(
                &mut tx,
                &user,
                story.title,
                story.url,
                Some(hydrant.id),
                Some(tag_selectors.clone()),
                now,
            )
            .await?;
        }

        sqlx::query!(
            "update hydrants set fetched_at = $1 where id = $2",
            now.naive_utc(),
            hydrant.id,
        )
        .execute(&mut tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
}

impl Hydrant {
    fn from_row(row: JoinHydrantsTagsRow) -> Self {
        Self {
            hydrant: row.hydrant(),
            tags: row.tag().into_iter().collect(),
        }
    }

    fn from_rows_one(rows: Vec<JoinHydrantsTagsRow>) -> Self {
        let v = Self::from_rows_vec(rows);
        v[0].clone()
    }

    fn from_rows_vec(rows: Vec<JoinHydrantsTagsRow>) -> Vec<Self> {
        let mut hydrant_ids: Vec<Uuid> = Vec::new();
        let mut hydrant_builds: HashMap<Uuid, Self> = HashMap::new();

        for row in rows {
            let hydrant_id = row.hydrant_id;

            match hydrant_builds.entry(hydrant_id) {
                Entry::Occupied(mut entry) => {
                    if let Some(tag) = row.tag() {
                        entry.get_mut().tags.push(tag);
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(Self::from_row(row));
                    hydrant_ids.push(hydrant_id);
                }
            }
        }

        let mut data: Vec<Self> = Vec::with_capacity(hydrant_ids.len());
        for id in hydrant_ids.drain(0..) {
            let hydrant = hydrant_builds.remove(&id).expect("id must be in map");
            data.push(hydrant);
        }
        data
    }
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct JoinHydrantsTagsRow {
    hydrant_id: Uuid,
    hydrant_user_id: Uuid,
    hydrant_name: String,
    hydrant_url: String,
    hydrant_active: bool,
    hydrant_tag_ids: Vec<Uuid>,
    hydrant_fetched_at: Option<Timestamp>,
    hydrant_created_at: Timestamp,
    hydrant_updated_at: Timestamp,

    tag_id: Option<Uuid>,
    tag_user_id: Option<Uuid>,
    tag_name: Option<String>,
    tag_color: Option<String>,
    tag_created_at: Option<Timestamp>,
    tag_updated_at: Option<Timestamp>,
}

impl JoinHydrantsTagsRow {
    fn select() -> PgQueryBuilder<'static> {
        QueryBuilder::new(
            "
            select

              hydrants.id         as hydrant_id
            , hydrants.user_id    as hydrant_user_id
            , hydrants.name       as hydrant_name
            , hydrants.url        as hydrant_url
            , hydrants.active     as hydrant_active
            , hydrants.tag_ids    as hydrant_tag_ids
            , hydrants.fetched_at as hydrant_fetched_at
            , hydrants.created_at as hydrant_created_at
            , hydrants.updated_at as hydrant_updated_at

            , tags.id         as tag_id
            , tags.user_id    as tag_user_id
            , tags.name       as tag_name
            , tags.color      as tag_color
            , tags.created_at as tag_created_at
            , tags.updated_at as tag_updated_at

            from hydrants
            left join tags on tags.id = ANY(hydrants.tag_ids)
            ",
        )
    }

    fn hydrant(&self) -> models::Hydrant {
        models::Hydrant {
            id: self.hydrant_id,
            user_id: self.hydrant_user_id,
            name: self.hydrant_name.clone(),
            url: self.hydrant_url.clone(),
            active: self.hydrant_active,
            tag_ids: self.hydrant_tag_ids.clone(),
            fetched_at: self.hydrant_fetched_at,
            created_at: self.hydrant_created_at,
            updated_at: self.hydrant_updated_at,
        }
    }

    fn tag(&self) -> Option<models::Tag> {
        self.tag_id?;

        Some(models::Tag {
            id: self.tag_id.unwrap(),
            user_id: self.tag_user_id.unwrap(),
            name: self.tag_name.as_ref().unwrap().clone(),
            color: self.tag_color.as_ref().unwrap().clone(),
            created_at: self.tag_created_at.unwrap(),
            updated_at: self.tag_updated_at.unwrap(),
        })
    }
}

pub async fn list_hydrants(
    conn: impl PgExecutor<'_>,
    user: &models::User,
) -> anyhow::Result<Vec<Hydrant>> {
    let mut query = JoinHydrantsTagsRow::select();
    query.push(" where hydrants.user_id = ");
    query.push_bind(user.id);
    query.push(
        "
        order by
            hydrants.name asc
          , hydrants.created_at asc
          , tags.name asc
        ",
    );

    let rows: Vec<JoinHydrantsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(Hydrant::from_rows_vec(rows))
}

pub async fn stale_hydrants(
    conn: &mut PgConnection,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<Hydrant>> {
    let mut query = JoinHydrantsTagsRow::select();
    query.push("where hydrants.active = true ");

    query.push(" and ( ");
    query.push(" hydrants.fetched_at is null ");
    query.push(" or hydrants.fetched_at < ");
    query.push_bind(now.naive_utc());
    query.push(" ) ");

    let rows: Vec<JoinHydrantsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(Hydrant::from_rows_vec(rows))
}

pub async fn find_hydrant(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    id: Uuid,
) -> anyhow::Result<Hydrant> {
    let user = user.clone();

    let mut query = JoinHydrantsTagsRow::select();
    query.push(" where hydrants.user_id = ");
    query.push_bind(user.id);
    query.push(" and hydrants.id = ");
    query.push_bind(id);
    query.push(
        "
        order by
            hydrants.name asc
          , hydrants.created_at asc
          , tags.name asc
        ",
    );

    let rows: Vec<JoinHydrantsTagsRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(Hydrant::from_rows_one(rows))
}

pub async fn create_hydrant(
    conn: &mut PgConnection,
    user: &models::User,
    name: &str,
    url: &str,
    active: bool, // TODO: Enum?
    tags: Option<Vec<TagSelector>>,
) -> sqlx::Result<Hydrant> {
    let user = user.clone();
    let name = name.to_string();
    let url = url.to_string();

    conn.transaction(|tx| {
        Box::pin(async move {
            let selectors = tags;
            let mut tags = Vec::new();
            let mut tag_ids = Vec::new();
            for sel in selectors.unwrap_or_default() {
                let tag = find_or_create_tag(&mut *tx, &user, sel).await?;
                tag_ids.push(tag.id);
                tags.push(tag);
            }

            let query = sqlx::query_as!(
                models::Hydrant,
                "
                insert into hydrants
                (user_id, name, url, active, tag_ids)
                values
                ($1, $2, $3, $4, $5)
                returning *
                ",
                user.id,
                name,
                url,
                active,
                &tag_ids,
            );

            let hydrant = query.fetch_one(&mut *tx).await?;

            tags.sort_by_key(|t| t.name.clone());
            Ok(Hydrant { hydrant, tags })
        })
    })
    .await
}

// TODO: Move *Fields to models?
#[derive(Default)]
pub struct HydrantFields {
    pub name: Option<String>,
    pub url: Option<String>,
    pub active: Option<bool>,
    pub tags: Option<Vec<TagSelector>>,
}

pub async fn update_hydrant(
    conn: &mut PgConnection,
    user: &models::User,
    hydrant: &models::Hydrant,
    fields: HydrantFields,
) -> anyhow::Result<Hydrant> {
    let user = user.clone();
    let hydrant = hydrant.clone();

    let mut query = QueryBuilder::new("update hydrants set");

    let mut assign = query.separated(" , ");
    let mut do_assign = false;
    if let Some(name) = fields.name {
        assign.push(" name = ");
        assign.push_bind_unseparated(name);
        do_assign = true;
    }
    if let Some(url) = fields.url {
        assign.push(" url = ");
        assign.push_bind_unseparated(url);
        do_assign = true;
    }
    if let Some(active) = fields.active {
        assign.push(" active = ");
        assign.push_bind_unseparated(active);
        do_assign = true;
    }

    conn.transaction(|tx| {
        Box::pin(async move {
            let tags = if let Some(selectors) = fields.tags {
                let mut tags = Vec::new();
                let mut tag_ids = Vec::new();
                for sel in selectors {
                    let tag = find_or_create_tag(&mut *tx, &user, sel).await?;
                    tag_ids.push(tag.id);
                    tags.push(tag);
                }

                // Lifetime issues with `query` and `assign` make it difficult to pass the builder
                // state into the closure. If we're already doing an assignment, push an empty
                // string onto the query to sync the separator state.
                let mut assign = query.separated(" , ");
                if do_assign {
                    assign.push("");
                }
                assign.push(" tag_ids = ");
                assign.push_bind_unseparated(tag_ids);
                do_assign = true;

                tags
            } else {
                find_tags(&mut *tx, &user, &hydrant.tag_ids).await?
            };

            query.push(" where id = ");
            query.push_bind(hydrant.id);
            query.push(" and user_id = ");
            query.push_bind(user.id);
            query.push(" returning *");

            if do_assign {
                let hydrant: models::Hydrant = query.build_query_as().fetch_one(&mut *tx).await?;
                Ok(Hydrant { hydrant, tags })
            } else {
                find_hydrant(&mut *tx, &user, hydrant.id).await
            }
        })
    })
    .await
}

pub async fn delete_hydrant(
    conn: impl PgExecutor<'_>,
    user: &models::User,
    hydrant: models::Hydrant,
) -> sqlx::Result<models::Hydrant> {
    sqlx::query_as!(
        models::Hydrant,
        "
        delete from hydrants
        where id = $1
        and user_id = $2
        returning *
        ",
        hydrant.id,
        user.id,
    )
    .fetch_one(conn)
    .await
}

#[cfg(test)]
mod tests {
    use chrono::SubsecRound;
    use sqlx::{Connection, PgConnection};

    use super::*;
    use crate::auth;

    // TODO: Make this a test transaction and roll it back on pass.
    async fn test_conn() -> sqlx::Result<PgConnection> {
        let url = std::env::var("TEST_DATABASE_URL").unwrap();
        PgConnection::connect(&url).await
    }

    async fn test_user(conn: &mut PgConnection) -> sqlx::Result<models::User> {
        let stytch_user_id: String = uuid::Uuid::new_v4().to_string();
        auth::create_user(conn, stytch_user_id).await
    }

    fn lorem_rss() -> anyhow::Result<url::Url> {
        Ok(std::env::var("LOREM_RSS_URL")?.parse()?)
    }

    #[tokio::test]
    async fn minimal_drop() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let created = create_drop(
            &mut tx,
            &user,
            None,
            "https://example.com/lorem-ipsum".to_string(),
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let found = find_drop(&mut tx, &user, created.drop.id).await.unwrap();
        assert_eq!(found, created);
    }

    #[tokio::test]
    async fn maximal_drop() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let coffee = create_tag(&mut tx, &user, "Coffee", "#c0ffee")
            .await
            .unwrap();

        let created = create_drop(
            &mut tx,
            &user,
            Some("Lorem Ipsum".to_string()),
            "https://example.com/lorem-ipsum".to_string(),
            None,
            Some(vec![
                TagSelector::Find { id: coffee.id },
                TagSelector::Create {
                    name: "ABC".to_string(),
                    color: models::Tag::DEFAULT_COLOR.to_string(),
                },
            ]),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let found = find_drop(&mut tx, &user, created.drop.id).await.unwrap();
        assert_eq!(found, created);

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["ABC", "Coffee"]);
    }

    #[tokio::test]
    async fn update_drop_fields() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let drop = create_drop(
            &mut tx,
            &user,
            None,
            "https://example.com/lorem-ipsum".to_string(),
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let fields = DropFields {
            title: Some("Dolor Sit".to_string()),
            url: Some("https://example.com/dolor-sit".to_string()),
        };
        let tags = None;

        let updated = update_drop(&mut tx, &user, &drop.drop, fields, tags)
            .await
            .unwrap();

        let found = find_drop(&mut tx, &user, drop.drop.id).await.unwrap();
        assert_eq!(found, updated);

        assert_eq!(found.drop.title, Some("Dolor Sit".to_string()));
        assert_eq!(found.drop.url, "https://example.com/dolor-sit".to_string(),);
    }

    #[tokio::test]
    async fn update_drop_tags() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let coffee = create_tag(&mut tx, &user, "Coffee", "#c0ffee")
            .await
            .unwrap();

        let drop = create_drop(
            &mut tx,
            &user,
            None,
            "https://example.com/lorem-ipsum".to_string(),
            None,
            Some(vec![TagSelector::Find { id: coffee.id }]),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let fields = Default::default();
        let tags = Some(vec![TagSelector::Create {
            name: "ABC".to_string(),
            color: models::Tag::DEFAULT_COLOR.to_string(),
        }]);

        let updated = update_drop(&mut tx, &user, &drop.drop, fields, tags)
            .await
            .unwrap();

        let found = find_drop(&mut tx, &user, drop.drop.id).await.unwrap();

        assert_eq!(found.drop, updated.drop);

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["ABC"]);
    }

    #[tokio::test]
    async fn change_drop_status() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let now = chrono::Utc::now();

        let drop = create_drop(
            &mut tx,
            &user,
            None,
            "https://example.com/lorem-ipsum".to_string(),
            None,
            None,
            now,
        )
        .await
        .unwrap();

        let drop_id = drop.drop.id;

        let moved = move_drop(
            &mut tx,
            drop,
            DropStatus::Read,
            now + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

        let found = find_drop(&mut tx, &user, drop_id).await.unwrap();
        assert_eq!(found, moved);

        // DB timestamps have microsecond precision, so truncate the local timestamp to the same
        // number of subsecond digits (6).
        assert_eq!(
            found.drop.moved_at,
            (now + chrono::Duration::minutes(5))
                .naive_utc()
                .trunc_subsecs(6)
        );
    }

    #[tokio::test]
    async fn list_drops_by_status() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();
        let now = chrono::Utc::now();

        let mut unread = Vec::new();
        let mut read = Vec::new();
        for i in 0..10 {
            let mut drop = create_drop(
                &mut tx,
                &user,
                None,
                format!("https://example.com/filtering/{}", i),
                None,
                None,
                now + chrono::Duration::seconds(i.try_into().unwrap()),
            )
            .await
            .unwrap();

            if i % 2 == 0 {
                drop = move_drop(&mut tx, drop, DropStatus::Read, now)
                    .await
                    .unwrap();
                read.push(drop);
            } else {
                unread.push(drop);
            }
        }

        let found_unread = list_drops(
            &mut tx,
            &user,
            DropFilters {
                status: Some(DropStatus::Unread),
                ..Default::default()
            },
            Some(100),
        )
        .await
        .unwrap();
        assert_eq!(found_unread, unread);
    }

    #[tokio::test]
    async fn list_drops_by_tags() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();
        let now = chrono::Utc::now();

        // Set up state that looks like this:
        //
        // D0: {T0}
        // D1: {T0, T1}
        // D2: {T0, T1, T2}

        let mut tags = Vec::new();
        for i in 0..3 {
            let tag = create_tag(&mut tx, &user, &i.to_string(), models::Tag::DEFAULT_COLOR)
                .await
                .unwrap();

            tags.push(tag);
        }

        let mut drops = Vec::new();
        for i in 0..3 {
            let sel = tags[0..i + 1]
                .iter()
                .cloned()
                .map(|tag| TagSelector::Find { id: tag.id })
                .collect();

            let drop = create_drop(
                &mut tx,
                &user,
                None,
                format!("https://example.com/filtering/{}", i),
                None,
                Some(sel),
                now + chrono::Duration::seconds(i.try_into().unwrap()),
            )
            .await
            .unwrap();

            assert_eq!(drop.tags.len(), i + 1);

            drops.push(drop);
        }

        // Each tag selects suffix slices from the drops:
        //
        // T0: {D0, D1, D2}
        // T1: {    D1, D2}
        // T2: {        D2}
        for i in 0..3 {
            let found = list_drops(
                &mut tx,
                &user,
                DropFilters {
                    tags: Some(vec![tags[i].clone()]),
                    ..Default::default()
                },
                Some(100),
            )
            .await
            .unwrap();

            assert_eq!(found.len(), 3 - i);

            let expected = &drops[i..];
            assert_eq!(found, expected);
        }
    }

    #[tokio::test]
    async fn update_tag_fields() {
        let mut conn = test_conn().await.unwrap();
        let mut conn = conn.begin().await.unwrap();

        let user = test_user(&mut conn).await.unwrap();

        let tag = create_tag(&mut conn, &user, "Work", "#0000ff")
            .await
            .unwrap();

        let tag_id = tag.id;

        let fields = TagFields {
            name: Some("Play".to_string()),
            color: Some("#00ff00".to_string()),
        };

        let updated = update_tag(&mut conn, &user, tag, fields).await.unwrap();

        let found = find_tag(&mut conn, &user, tag_id).await.unwrap();
        assert_eq!(found, updated);

        assert_eq!(found.name, "Play".to_string());
        assert_eq!(found.color, "#00ff00".to_string());
    }

    #[tokio::test]
    async fn empty_stream() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let res = create_stream(&mut tx, &user, "Empty", &[]).await;
        let err = res.unwrap_err();
        assert!(matches!(err, Error::Stream(StreamError::NoTags)));
    }

    #[tokio::test]
    async fn stream_with_tags() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let red = create_tag(&mut tx, &user, "Red", "#ff0000").await.unwrap();

        let blue = create_tag(&mut tx, &user, "Blue", "#0000ff").await.unwrap();

        let created = create_stream(&mut tx, &user, "Colors", &vec![red, blue])
            .await
            .unwrap();

        let found = find_stream(&mut tx, &user, created.stream.id)
            .await
            .unwrap();
        assert_eq!(created, found);

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["Blue", "Red"]);
    }

    #[tokio::test]
    async fn change_stream_fields() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let red = create_tag(&mut tx, &user, "Red", "#ff0000").await.unwrap();

        let stream = create_stream(&mut tx, &user, "Oops!", &[red])
            .await
            .unwrap();

        let green = create_tag(&mut tx, &user, "Green", "#00ff00")
            .await
            .unwrap();

        let fields = StreamFields {
            name: Some("Yay!".to_string()),
            tag_ids: Some(vec![green.id]),
        };

        let updated = update_stream(&mut tx, &user, &stream.stream, fields)
            .await
            .unwrap();

        let found = find_stream(&mut tx, &user, stream.stream.id).await.unwrap();
        assert_eq!(updated, found);

        assert_eq!(found.stream.name, "Yay!".to_string());

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["Green"]);
    }

    #[tokio::test]
    async fn list_streams_default() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let found = list_streams(&mut tx, &user).await.unwrap();

        let expected = vec![
            Stream::Status(StatusStream::new(DropStatus::Unread)),
            Stream::Status(StatusStream::new(DropStatus::Read)),
            Stream::Status(StatusStream::new(DropStatus::Saved)),
        ];

        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn list_streams_custom() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let red = create_tag(&mut tx, &user, "Red", "#ff0000").await.unwrap();

        let blue = create_tag(&mut tx, &user, "Blue", "#0000ff").await.unwrap();

        let colors = create_stream(&mut tx, &user, "Colors", &vec![red, blue.clone()])
            .await
            .unwrap();

        let only_blue = create_stream(&mut tx, &user, "Only Blue", &[blue.clone()])
            .await
            .unwrap();

        let found = list_streams(&mut tx, &user).await.unwrap();

        let expected = vec![
            Stream::Status(StatusStream::new(DropStatus::Unread)),
            Stream::Status(StatusStream::new(DropStatus::Read)),
            Stream::Status(StatusStream::new(DropStatus::Saved)),
            Stream::Custom(CustomStream { ..colors }),
            Stream::Custom(CustomStream { ..only_blue }),
        ];
        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn simple_hydrant() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let created = create_hydrant(
            &mut tx,
            &user,
            "Simple",
            "https://example.com/simple",
            true,
            None,
        )
        .await
        .unwrap();

        let found = find_hydrant(&mut tx, &user, created.hydrant.id)
            .await
            .unwrap();
        assert_eq!(created, found);

        assert_eq!(found.hydrant.name, "Simple".to_string());
        assert_eq!(found.hydrant.url, "https://example.com/simple".to_string());
        assert!(found.hydrant.active);
        assert_eq!(found.tags, vec![]);
    }

    #[tokio::test]
    async fn tagged_hydrant() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let red = create_tag(&mut tx, &user, "Red", "#ff0000").await.unwrap();

        let created = create_hydrant(
            &mut tx,
            &user,
            "Painted",
            "https://example.com/painted",
            true,
            Some(vec![
                TagSelector::Find { id: red.id },
                TagSelector::Create {
                    name: "Blue".to_string(),
                    color: "#0000ff".to_string(),
                },
            ]),
        )
        .await
        .unwrap();

        let found = find_hydrant(&mut tx, &user, created.hydrant.id)
            .await
            .unwrap();
        assert_eq!(created, found);

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["Blue", "Red"]);
    }

    #[tokio::test]
    async fn change_hydrant_fields() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let black = create_tag(&mut tx, &user, "Black", "#ff0000")
            .await
            .unwrap();

        let hydrant = create_hydrant(
            &mut tx,
            &user,
            "The Rolling Stones",
            "https://example.com/painted",
            true,
            Some(vec![TagSelector::Find { id: black.id }]),
        )
        .await
        .unwrap();

        let blue = create_tag(&mut tx, &user, "Blue", "#0000ff").await.unwrap();

        let fields = HydrantFields {
            name: Some("Eiffel 65".to_string()),
            url: Some("https://example.com/blue".to_string()),
            active: Some(false),
            tags: Some(vec![TagSelector::Find { id: blue.id }]),
        };

        let updated = update_hydrant(&mut tx, &user, &hydrant.hydrant, fields)
            .await
            .unwrap();

        let found = find_hydrant(&mut tx, &user, hydrant.hydrant.id)
            .await
            .unwrap();
        assert_eq!(updated, found);

        assert_eq!(found.hydrant.name, "Eiffel 65".to_string());
        assert_eq!(found.hydrant.url, "https://example.com/blue".to_string());
        assert!(!found.hydrant.active);

        let tag_names: Vec<&str> = found.tags.iter().map(|t| &t.name[..]).collect();
        assert_eq!(tag_names, vec!["Blue"]);
    }

    #[tokio::test]
    async fn list_hydrants_empty() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let found = list_hydrants(&mut tx, &user).await.unwrap();
        assert_eq!(found, vec![]);
    }

    #[tokio::test]
    async fn list_hydrants_custom() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let red = create_tag(&mut tx, &user, "Red", "#ff0000").await.unwrap();

        let blue = create_tag(&mut tx, &user, "Blue", "#0000ff").await.unwrap();

        let painted = create_hydrant(
            &mut tx,
            &user,
            "Painted",
            "https://example.com/painted",
            true,
            Some(vec![
                TagSelector::Find { id: red.id },
                TagSelector::Find { id: blue.id },
            ]),
        )
        .await
        .unwrap();

        let only_blue = create_hydrant(
            &mut tx,
            &user,
            "Only Blue",
            "https://example.com/blue",
            true,
            Some(vec![TagSelector::Find { id: blue.id }]),
        )
        .await
        .unwrap();

        let found = list_hydrants(&mut tx, &user).await.unwrap();

        let expected = vec![only_blue, painted];
        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn stale_hydrants_list() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let url = lorem_rss().unwrap().join("feed").unwrap();

        let now = chrono::Utc::now();
        let client = reqwest::Client::new();

        let found = stale_hydrants(&mut tx, now).await.unwrap();
        assert_eq!(found, vec![]);

        // Leave this one unfetched.
        let new = create_hydrant(&mut tx, &user, "New", url.as_ref(), true, None)
            .await
            .unwrap();

        let stale = {
            let hydrant = create_hydrant(&mut tx, &user, "Stale", url.as_ref(), true, None)
                .await
                .unwrap();
            Hydrant::fetch(
                &mut tx,
                &client,
                hydrant.hydrant.id,
                now - chrono::Duration::minutes(1),
            )
            .await
            .unwrap();

            find_hydrant(&mut tx, &user, hydrant.hydrant.id)
                .await
                .unwrap()
        };

        let _fresh = {
            let hydrant = create_hydrant(&mut tx, &user, "Fresh", url.as_ref(), true, None)
                .await
                .unwrap();
            Hydrant::fetch(
                &mut tx,
                &client,
                hydrant.hydrant.id,
                // Avoid sub-second equality issues by ensuring that this one's fetched_at is
                // strictly greater than now.
                now + chrono::Duration::seconds(1),
            )
            .await
            .unwrap();

            find_hydrant(&mut tx, &user, hydrant.hydrant.id)
                .await
                .unwrap()
        };

        let _inactive = create_hydrant(&mut tx, &user, "Inactive", url.as_ref(), false, None)
            .await
            .unwrap();

        let found = stale_hydrants(&mut tx, now).await.unwrap();

        let expected = vec![new, stale];
        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn fetch_skips_inactive_hydrant() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let url = lorem_rss().unwrap().join("feed").unwrap();

        let now = chrono::Utc::now();
        let client = reqwest::Client::new();

        let hydrant = create_hydrant(&mut tx, &user, "Inactive", url.as_ref(), false, None)
            .await
            .unwrap();

        Hydrant::fetch(&mut tx, &client, hydrant.hydrant.id, now)
            .await
            .unwrap();

        let found = find_hydrant(&mut tx, &user, hydrant.hydrant.id)
            .await
            .unwrap();

        assert_eq!(found.hydrant.fetched_at, None);
    }

    #[tokio::test]
    async fn fetch_empty_rss() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let mut url = lorem_rss().unwrap();
        url = url.join("feed").unwrap();
        url.set_query(Some("length=0"));

        let now = chrono::Utc::now();
        let client = reqwest::Client::new();

        let hydrant = create_hydrant(&mut tx, &user, "Empty", url.as_ref(), true, None)
            .await
            .unwrap();

        Hydrant::fetch(&mut tx, &client, hydrant.hydrant.id, now)
            .await
            .unwrap();

        let found = find_hydrant(&mut tx, &user, hydrant.hydrant.id)
            .await
            .unwrap();

        // DB timestamps have microsecond precision, so truncate the local timestamp to the same
        // number of subsecond digits (6).
        assert_eq!(
            found.hydrant.fetched_at,
            Some(now.naive_utc().trunc_subsecs(6))
        );
    }

    #[tokio::test]
    async fn fetch_small_rss() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        let mut url = lorem_rss().unwrap();
        url = url.join("feed").unwrap();
        url.set_query(Some("length=10"));

        let now = chrono::Utc::now();

        let client = reqwest::Client::new();

        let hydrant = create_hydrant(&mut tx, &user, "10 items/minute", url.as_ref(), true, None)
            .await
            .unwrap();

        Hydrant::fetch(&mut tx, &client, hydrant.hydrant.id, now)
            .await
            .unwrap();

        let found = find_hydrant(&mut tx, &user, hydrant.hydrant.id)
            .await
            .unwrap();

        // DB timestamps have microsecond precision, so truncate the local timestamp to the same
        // number of subsecond digits (6).
        assert_eq!(
            found.hydrant.fetched_at,
            Some(now.naive_utc().trunc_subsecs(6))
        );

        let drops = list_drops(
            &mut tx,
            &user,
            DropFilters {
                status: Some(DropStatus::Unread),
                ..Default::default()
            },
            Some(100),
        )
        .await
        .unwrap();

        assert_eq!(drops.len(), 10);
    }

    #[tokio::test]
    async fn fetch_skips_seen_items() {
        let mut conn = test_conn().await.unwrap();
        let mut tx = conn.begin().await.unwrap();

        let user = test_user(&mut tx).await.unwrap();

        // Pick a long interval (once per month) to make finding a new item less likely.
        let mut url = lorem_rss().unwrap();
        url = url.join("feed").unwrap();
        url.set_query(Some("unit=month&length=5"));

        let now = chrono::Utc::now();

        let client = reqwest::Client::new();

        let hydrant = create_hydrant(&mut tx, &user, "5 items/month", url.as_ref(), true, None)
            .await
            .unwrap();

        for _ in 0..2 {
            let now = now + chrono::Duration::minutes(1);

            Hydrant::fetch(&mut tx, &client, hydrant.hydrant.id, now)
                .await
                .unwrap();

            let found = find_hydrant(&mut tx, &user, hydrant.hydrant.id)
                .await
                .unwrap();

            // DB timestamps have microsecond precision, so truncate the local timestamp to the same
            // number of subsecond digits (6).
            assert_eq!(
                found.hydrant.fetched_at,
                Some(now.naive_utc().trunc_subsecs(6))
            );

            let drops = list_drops(
                &mut tx,
                &user,
                DropFilters {
                    status: Some(DropStatus::Unread),
                    ..Default::default()
                },
                Some(100),
            )
            .await
            .unwrap();

            assert_eq!(drops.len(), 5);
        }
    }
}
