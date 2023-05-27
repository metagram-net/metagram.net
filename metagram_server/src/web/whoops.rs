use axum::Router;
use axum_extra::routing::{RouterExt, TypedPath};
use serde::Deserialize;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .typed_get(internal_server_error)
        .typed_get(unprocessable_entity)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/whoops/500")]
pub struct InternalServerError;

pub async fn internal_server_error(_: InternalServerError) -> super::Result<()> {
    Err(anyhow::anyhow!("Hold my beverage!").into())
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/whoops/422")]
pub struct UnprocessableEntity;

pub async fn unprocessable_entity(_: UnprocessableEntity) -> super::Result<()> {
    Err(super::Error::CsrfMismatch {
        cookie: "cookie_token".to_string(),
        form: "form_token".to_string(),
    })
}

// We should get /whoops/404 for free 😉 But for searchability: NotFound, not_found
