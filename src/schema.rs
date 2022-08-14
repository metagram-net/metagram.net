table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    drop_tags (id) {
        id -> Uuid,
        drop_id -> Uuid,
        tag_id -> Uuid,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    drops (id) {
        id -> Uuid,
        user_id -> Uuid,
        title -> Nullable<Text>,
        url -> Text,
        status -> Drop_status,
        moved_at -> Timestamp,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    hydrants (id) {
        id -> Uuid,
        user_id -> Uuid,
        name -> Text,
        url -> Text,
        active -> Bool,
        tag_ids -> Array<Uuid>,
        fetched_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    streams (id) {
        id -> Uuid,
        user_id -> Uuid,
        name -> Text,
        tag_ids -> Array<Uuid>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    tags (id) {
        id -> Uuid,
        user_id -> Uuid,
        name -> Text,
        color -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    users (id) {
        id -> Uuid,
        stytch_user_id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

joinable!(drop_tags -> drops (drop_id));
joinable!(drop_tags -> tags (tag_id));
joinable!(drops -> users (user_id));
joinable!(hydrants -> users (user_id));
joinable!(streams -> users (user_id));
joinable!(tags -> users (user_id));

allow_tables_to_appear_in_same_query!(
    drop_tags,
    drops,
    hydrants,
    streams,
    tags,
    users,
);
