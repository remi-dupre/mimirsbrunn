use crate::prometheus_middleware;
use crate::routes::{
    autocomplete, entry_point, features, metrics, post_autocomplete, reverse, status,
};
use crate::{Args, Context};
use actix_web::{http, middleware, server, App};
use structopt::StructOpt;
use crate::extractors::ActixError;

pub fn create_server(ctx: Context) -> App<Context> {
    App::with_state(ctx)
        .middleware(
            middleware::cors::Cors::build()
                .allowed_methods(vec!["GET"])
                .finish(),
        )
        .middleware(middleware::Logger::default())
        .middleware(prometheus_middleware::PrometheusMiddleware::default())
        .resource("/", |r| r.f(entry_point))
        .resource("/autocomplete", |r| {
            r.method(http::Method::GET).with(autocomplete);
            r.method(http::Method::POST)
                .with_config(post_autocomplete, |(_, _, json_cfg)| {
                    json_cfg.error_handler(|err, _req| {
                        ActixError::InvalidJson(format!("{}", err)).into()
                    });
                });
        })
        .resource("/status", |r| r.with(status))
        .resource("/features/{id}", |r| r.with(features))
        .resource("/reverse", |r| r.with(reverse))
        .resource("/metrics", |r| r.f(metrics))
}

pub fn runserver() {
    let args = Args::from_args();
    let ctx: Context = (&args).into();
    server::new(move || create_server(ctx.clone()))
        .bind(&args.bind)
        .unwrap()
        .run();
}
