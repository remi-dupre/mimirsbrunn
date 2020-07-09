use crate::extractors::BragiQuery;
use crate::routes::params;
use crate::{model, model::FromWithLang, Context};
use actix_http::http::header::{CacheControl, CacheDirective};
use actix_web::web::{Data, HttpResponse};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Params {
    lat: f64,
    lon: f64,
    /// timeout in milliseconds
    timeout: Option<u64>,
}

pub async fn reverse(
    params: BragiQuery<Params>,
    state: Data<Context>,
) -> Result<HttpResponse, model::BragiError> {
    let mut rubber = state.get_rubber_for_reverse(params.timeout.map(Duration::from_millis));
    let coord = params::make_coord(params.lon, params.lat)?;
    rubber
        .get_address(&coord)
        .map_err(model::BragiError::from)
        .map(|r| model::Autocomplete::from_with_lang(r, None))
        .map(|v| {
            HttpResponse::Ok()
                .set(CacheControl(vec![CacheDirective::MaxAge(
                    state.http_cache_duration,
                )]))
                .json(v)
        })
}
