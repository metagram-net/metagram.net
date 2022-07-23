use diesel::{QueryId, SqlType};

#[derive(Debug, Clone, Copy, Default, SqlType, QueryId)]
#[diesel(postgres_type(name = "drop_status"))]
#[allow(non_camel_case_types)]
pub struct Drop_status;
