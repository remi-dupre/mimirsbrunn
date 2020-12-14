use std::future::Future;

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut rt = tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build()
        .expect("failed to init tokio runtime");

    rt.block_on(future)
}

// ---

#[derive(serde::Deserialize, Debug)]
pub struct Results {
    hits: Hits,
    took: u32,
}

#[derive(serde::Deserialize, Debug)]
struct Hits {
    hits: Vec<Hit>,
    total: usize,
}

#[derive(serde::Deserialize, Debug)]
struct Hit {
    #[serde(rename = "_source")]
    source: serde_json::Value,
    #[serde(rename = "_type")]
    doc_type: String,
    #[serde(rename = "_explanation")]
    explanation: Option<serde_json::Value>,
}

pub fn read_places_es7(
    result: Results,
    coord: Option<&mimir::Coord>, // coord used to compute the distance of the place to the object
) -> Vec<mimir::Place> {
    slog_scope::debug!(
        "{} documents found in {} ms",
        result.hits.total,
        result.took
    );
    let point: Option<geo_types::Point<f64>> = coord.map(|c| c.0.into());
    // TODO: handle enum properly?
    result
        .hits
        .hits
        .into_iter()
        .filter_map(|hit| {
            mimir::rubber::make_place(hit.doc_type, Some(Box::new(hit.source)), hit.explanation)
        })
        .map(|mut place| {
            if let Some(ref p) = point {
                use geo::algorithm::haversine_distance::HaversineDistance;
                let distance = p.haversine_distance(&place.coord().0.into()) as u32;
                place.set_distance(distance);
            }
            place
        })
        .collect()
}
