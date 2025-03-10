#![allow(unused)]

pub mod http;
pub mod iiif;
pub mod image;
pub mod runtime;
pub mod telemetry;

use std::env;
use std::iter::once;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use http::IiifImageService;
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, COOKIE};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::service::TowerToHyperService;
use kaduceus::KakaduContext;
use mediatype::{MediaType, MediaTypeBuf};
use opentelemetry::trace::FutureExt;
use opentelemetry::Context;
use opentelemetry_sdk::trace::Span;
use runtime::tokio::TokioRuntime;
use runtime::Runtime;
use serde::{Deserialize, Serialize};
use tower::{ServiceBuilder, ServiceExt};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::classify::StatusInRangeAsFailures;
use tower_http::cors::{Any, CorsLayer};
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::request_id::PropagateRequestIdLayer;
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::field::Empty;
use tracing::{info, info_span, Level};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_opentelemetry_instrumentation_sdk::http::http_server::{
    make_span_from_request, update_span_from_response,
};
use tracing_opentelemetry_instrumentation_sdk::http::{
    http_flavor, http_host, http_method, url_scheme, user_agent,
};
use tracing_opentelemetry_instrumentation_sdk::otel_trace_span;

use crate::iiif::Format;
use crate::image::metadata::KaduceusImageReader;
use crate::image::{ImagePipelineBuilder, LocalImageSourceResolver};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ImageSourceOptions {
    Local { path: Box<Path> },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IiifImageServiceOptions {
    prefix: String,
    default_format: String,
    image_source: ImageSourceOptions,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HttpOptions {
    bind_address: SocketAddr,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Options {
    http: HttpOptions,
    iiif: IiifImageServiceOptions,
}

pub fn start<R: Runtime>(rt: R, options: LayaOptions) {
    let kdu_context = KakaduContext::default();
    let image_pipeline = ImagePipelineBuilder::new()
        .with_locator(LocalImageSourceResolver::new("samples"))
        .with_reader(KaduceusImageReader::new(kdu_context))
        .build();
    let image_service = IiifImageService::new_with_prefix(image_pipeline, &options.prefix);

    fn on_response<B>(response: &Response<B>, latency: Duration, span: &tracing::Span) {}

    let svc = ServiceBuilder::new()
        .layer(SetSensitiveRequestHeadersLayer::new([AUTHORIZATION, COOKIE]))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request<Incoming>| {
                    let http_method = http_method(req.method());
                    otel_trace_span!(
                        parent: None,
                        "HTTP request",
                        http.request.method = %http_method,
                        network.protocol.version = %http_flavor(req.version()),
                        server.address = http_host(req),
                        // server.port = req.uri().port(),
                        http.client.address = Empty, //%$request.connection_info().realip_remote_addr().unwrap_or(""),
                        user_agent.original = user_agent(req),
                        http.response.status_code = Empty, // to set on response
                        url.path = req.uri().path(),
                        url.query = req.uri().query(),
                        url.scheme = url_scheme(req.uri()),
                        otel.name = %http_method, // to set by router of "webframework" after
                        otel.kind = ?opentelemetry::trace::SpanKind::Server,
                        otel.status_code = Empty, // to set on response
                        trace_id = Empty, // to set on response
                        request_id = Empty, // to set
                        exception.message = Empty, // to set on response
                    )
                })
                .on_response(|response: &Response<_>, latency: Duration, span: &tracing::Span| {
                    update_span_from_response(span, response)
                }),
        )
        .layer(TimeoutLayer::new(Duration::from_secs(10)))
        .service(image_service);

    R::bind(options, TowerToHyperService::new(svc));
}

#[derive(Clone, Default, Debug, clap::ValueEnum)]
pub enum Rt {
    #[cfg(all(feature = "rt-glommio", target_os = "linux"))]
    Glommio,
    #[cfg(feature = "rt-tokio")]
    #[default]
    Tokio,
}

#[derive(clap::Parser, Debug)]
pub struct LayaOptions {
    /// Use a Tokio multi-threaded runtime to handle IO and server traffic.
    #[arg(long, conflicts_with_all(["glommio"]), help_heading("Runtime"))]
    tokio: bool,

    /// Use a Glommio thread-per-core runtime to handle IO and server traffic.
    #[arg(long, conflicts_with_all(["tokio"]), help_heading("Runtime"))]
    glommio: bool,

    /// Network address the HTTP server is bound to.
    #[arg(long, short, default_value("127.0.0.1:43594"))]
    bind_address: SocketAddr,

    /// Prefix expected on any image requests.
    #[arg(long, default_value("/"))]
    prefix: String,

    /// Default media-type of encoded images when no preference is specified by the client.
    #[arg(long, default_value("image/jpeg"))]
    default_image_format: String,

    /// Run Laya in development mode?
    #[arg(long, default_missing_value("true"))]
    dev: bool,

    #[command(flatten)]
    tokio_options: TokioRuntimeOptions,

    #[command(flatten)]
    image_decoder_options: ImageDecoderOptions,
}

#[derive(clap::Args, Clone, Debug)]
pub struct ImageDecoderOptions {
    #[command(flatten)]
    kakadu: KakaduOptions,
}

#[derive(clap::Args, Clone, Debug)]
pub struct KakaduOptions {
    /// The number of threads used to run image decoding operations. Defaults to the number of
    /// available cores.
    #[arg(long("kakadu-decoder-threads"), help_heading("Kakadu"))]
    decoder_threads: Option<usize>,

    /// The amount of memory allocated to Kakadu in bytes.
    /// NOTE: failure to assign an upper bound on memory allocations may result in Kakadu panicking
    /// during an image decode request.
    #[arg(long("kakadu-memory-limit"), help_heading("Kakadu"))]
    memory_limit: Option<usize>,
}

#[derive(clap::Args, Clone, Debug)]
pub struct TokioRuntimeOptions {
    /// How many threads should be allocated to HTTP listener sockets?
    #[arg(
        long("tokio-listener-threads"),
        requires("tokio"),
        help_heading("Tokio"),
        default_value("1")
    )]
    listener_threads: usize,

    /// How many threads should be allocated to file and network IO?
    #[arg(
        long("tokio-io-threads"),
        requires("tokio"),
        help_heading("Tokio"),
        default_value("1")
    )]
    io_threads: usize,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let telemetry_rt = telemetry::install_telemetry_collector();
    let options = LayaOptions::parse();

    let rt = Rt::Tokio;

    info_span!("main", runtime = ?rt, options = ?options).in_scope(|| {
        match rt {
            #[cfg(all(feature = "rt-glommio", target_os = "linux"))]
            Rt::Glommio => {
                todo!()
            }

            #[cfg(feature = "rt-tokio")]
            Rt::Tokio => start(TokioRuntime, options),
        }

        telemetry_rt.shutdown(Duration::from_secs(5));

        Ok(())
    })
}
