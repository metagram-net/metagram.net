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

    users (id) {
        id -> Uuid,
        stytch_user_id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

joinable!(drops -> users (user_id));

allow_tables_to_appear_in_same_query!(drops, users,);
