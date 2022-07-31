use axum::{
    routing::{get, post},
    Router,
};
use axum_extra::routing::RouterExt;

use crate::controllers;

pub fn build() -> Router {
    use controllers::*;

    Router::new()
        .route("/", get(home::index))
        .route("/.well-known/health-check", get(home::health_check))
        .route("/auth/login", get(auth::login).post(auth::login_form))
        .route("/auth/logout", post(auth::logout))
        .route(
            "/auth/authenticate",
            get(auth::authenticate).head(auth::authenticate_head),
        )
        .route("/firehose", get(firehose::index))
        .route("/firehose/about", get(firehose::about))
        .route("/firehose/streams/:id", get(streams::show))
        .typed_get(drops::index)
        .typed_get(drops::new)
        .typed_post(drops::create)
        .typed_get(drops::show)
        .typed_get(drops::edit)
        .typed_post(drops::update)
        .typed_post(drops::r#move)
        .route("/whoops/500", get(errors::internal_server_error))
        .route("/whoops/422", get(errors::unprocessable_entity))
}

#[cfg(test)]
mod tests {
    #[test]
    fn build() {
        super::build();
    }
}
