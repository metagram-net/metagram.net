use axum::response::Response;

use crate::{AppError, Context, Session};

pub async fn internal_server_error(context: Context, session: Option<Session>) -> Response {
    let err = anyhow::anyhow!("Hold my beverage!");
    context.error(session, err.into())
}

pub async fn unprocessable_entity(context: Context, session: Option<Session>) -> Response {
    let err = AppError::CsrfMismatch;
    context.error(session, err)
}
