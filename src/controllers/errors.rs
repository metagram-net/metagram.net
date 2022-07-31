use axum::response::Response;
use axum_extra::routing::TypedPath;
use serde::Deserialize;

use crate::{AppError, Context, Session};

#[derive(TypedPath, Deserialize)]
#[typed_path("/whoops/500")]
pub struct InternalServerError;

pub async fn internal_server_error(
    _: InternalServerError,
    context: Context,
    session: Option<Session>,
) -> Response {
    let err = anyhow::anyhow!("Hold my beverage!");
    context.error(session, err.into())
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/whoops/422")]
pub struct UnprocessableEntity;

pub async fn unprocessable_entity(
    _: UnprocessableEntity,
    context: Context,
    session: Option<Session>,
) -> Response {
    let err = AppError::CsrfMismatch;
    context.error(session, err)
}
