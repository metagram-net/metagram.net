use axum::Router;
use axum_extra::routing::RouterExt;

use crate::controllers;

pub fn build() -> Router {
    use controllers::*;

    Router::new()
        .typed_get(home::index)
        .typed_get(home::about)
        .typed_get(home::health_check)
        .typed_get(auth::login)
        .typed_post(auth::login_form)
        .typed_post(auth::logout)
        .typed_get(auth::authenticate)
        .typed_head(auth::authenticate_head)
        .typed_get(firehose::index)
        .typed_get(firehose::about)
        .typed_get(drops::index)
        .typed_get(drops::new)
        .typed_post(drops::create)
        .typed_get(drops::show)
        .typed_get(drops::edit)
        .typed_post(drops::update)
        .typed_post(drops::r#move)
        .typed_get(hydrants::index)
        .typed_get(hydrants::new)
        .typed_post(hydrants::create)
        .typed_get(hydrants::show)
        .typed_get(hydrants::edit)
        .typed_post(hydrants::update)
        .typed_get(streams::index)
        .typed_get(streams::new)
        .typed_post(streams::create)
        .typed_get(streams::show)
        .typed_get(streams::edit)
        .typed_post(streams::update)
        .typed_get(tags::index)
        .typed_get(tags::new)
        .typed_post(tags::create)
        .typed_get(tags::show)
        .typed_get(tags::edit)
        .typed_post(tags::update)
        .typed_get(errors::internal_server_error)
        .typed_get(errors::unprocessable_entity)
}

#[cfg(test)]
mod tests {
    #[test]
    fn build() {
        super::build();
    }
}
