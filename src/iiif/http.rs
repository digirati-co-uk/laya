use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::task::Poll;

use futures::StreamExt;
use http::uri::PathAndQuery;
use http_body::Frame;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_TYPE, HeaderValue, IF_MODIFIED_SINCE, LAST_MODIFIED};
use hyper::{Request, Response, StatusCode, Uri};
use mediatype::MediaType;
use mediatype::names::{PLAIN, TEXT};
use serde_json::{Value, json, to_string_pretty};
use tower::Service;
use tracing::{Instrument, error, info};

use super::service::{
    ImageServiceError, ImageServiceRequestKind, ImageServiceResponse, ImageServiceResponseKind,
};
use crate::iiif::ImageServiceRequest;
use crate::iiif::parse::ParseError as ImageRequestParseError;
use crate::storage::StorageError;

#[derive(Clone)]
pub struct HttpImageService<S>
where
    S: Clone,
{
    inner: S,
    prefix: String,
}

impl<S: Clone> HttpImageService<S> {
    pub(crate) fn new_with_prefix(image_service: S, prefix: &str) -> Self {
        Self { inner: image_service, prefix: prefix.to_string() }
    }
}

impl<S> tower::Service<Request<Incoming>> for HttpImageService<S>
where
    S: Service<ImageServiceRequest, Response = ImageServiceResponse, Error = ImageServiceError>
        + Send
        + Sync
        + Clone
        + 'static,
    S::Future: Send + Unpin,
{
    type Response = Response<BoxBody<Bytes, std::io::Error>>;
    type Error = hyper::http::Error;
    type Future = Pin<Box<dyn Send + Future<Output = Result<Self::Response, Self::Error>>>>;

    fn call(&mut self, req: Request<Incoming>) -> Self::Future {
        Box::pin(Self::decode_request(req, self.prefix.clone(), self.inner.clone()))
    }

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

type HttpImageServiceBody = BoxBody<Bytes, std::io::Error>;
type HttpImageServiceResponse = Response<HttpImageServiceBody>;

const IMAGE_REQUEST_ROUTE: &str =
    "/<prefix>/<identifier>/<region>/<size>/<rotation>/<quality>.<format>";
const INFO_REQUEST_ROUTE: &str = "/<prefix>/<identifier>/info.json";

impl<S> HttpImageService<S>
where
    S: Service<ImageServiceRequest, Response = ImageServiceResponse, Error = ImageServiceError>
        + Send
        + Sync
        + Clone
        + 'static,
    S::Future: Send,
{
    pub async fn decode_request(
        req: Request<Incoming>,
        prefix: String,
        mut inner: S,
    ) -> Result<HttpImageServiceResponse, hyper::http::Error> {
        info!("Decoding request for prefix {prefix}, path: {}", req.uri().path());

        let request_path = req
            .uri()
            .path()
            .trim_start_matches(prefix.trim_end_matches("/"))
            .to_string();

        info!("Requested path: {request_path}");

        let request_span = tracing::Span::current();
        let request_method = req.method().to_string();
        let request = match request_path.as_str() {
            "/" => return text_response(StatusCode::OK, "OK!"),
            "/favicon.ico" => return text_response(StatusCode::NOT_FOUND, "File not found!"),
            _ => {
                let last_access_time = req
                    .headers()
                    .get(IF_MODIFIED_SINCE)
                    .and_then(|value| httpdate::parse_http_date(value.to_str().ok()?).ok());

                request_path
                    .parse::<ImageServiceRequest>()
                    .map(|req| req.with_last_access_time(last_access_time))
            }
        };

        match request {
            Ok(request) => {
                let route = match &request {
                    ImageServiceRequest { kind: ImageServiceRequestKind::Info, .. } => {
                        INFO_REQUEST_ROUTE
                    }
                    ImageServiceRequest { kind: ImageServiceRequestKind::Image(..), .. } => {
                        IMAGE_REQUEST_ROUTE
                    }
                };

                request_span.record("otel.name", format!("{} {route}", request_method));

                let image_id = request.identifier.clone();

                match inner.call(request).instrument(request_span).await {
                    Ok(response) => iiif_response(&image_id, req.uri(), response),
                    Err(ImageServiceError::Storage(StorageError::NotFound)) => {
                        text_response(StatusCode::NOT_FOUND, "Image file not found")
                    }
                    Err(ImageServiceError::ReaderUnsupported(ty)) => Response::builder()
                        .status(StatusCode::NOT_IMPLEMENTED)
                        .header(CONTENT_TYPE, MediaType::new(TEXT, PLAIN).to_string())
                        .body(text_body(format!("No readers found for {}", ty.as_str()))),
                    Err(e) => {
                        error!("failed to handle an image service request: {e:?}");

                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(text_body("An internal error occurred"))
                    }
                }
            }
            Err(e) => Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(text_body(e.to_string())),
        }
    }
}

pub fn iiif_response(
    image_id: &str,
    original_request_uri: &Uri,
    response: ImageServiceResponse,
) -> Result<HttpImageServiceResponse, hyper::http::Error> {
    let mut builder = Response::builder();
    let headers = builder.headers_mut().unwrap();

    if let Some(Ok(value)) = response
        .last_modified_time
        .map(httpdate::fmt_http_date)
        .map(|value| HeaderValue::from_str(&value))
    {
        headers.append(LAST_MODIFIED, value);
    }

    match response.kind {
        ImageServiceResponseKind::CacheHit => Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .body(BodyExt::boxed(Empty::new().map_err(|_| unreachable!()))),

        ImageServiceResponseKind::Image(image) => {
            let body = StreamBody::new(image.data.map(|data| data.map(Frame::data)));

            builder
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, image.media_type.canonicalize().to_string())
                .body(BodyExt::boxed(body))
        }
        ImageServiceResponseKind::Info(info) => {
            let mut id_parts = original_request_uri.clone().into_parts();
            id_parts.path_and_query = Some(format!("/{image_id}").parse()?);

            let id = Uri::from_parts(id_parts)?;

            let mut document = json!({
                "@context": "http://iiif.io/api/image/3/context.json",
                "type": "ImageService3",
                "id": id.to_string(),
                "protocol": "http://iiif.io/api/image",
                "profile": "level0",
                "width": info.width,
                "height": info.height,
            });

            if let Some(sizes) = &info.sizes {
                let sizes_documents: Vec<Value> = sizes
                    .iter()
                    .map(|size| {
                        json!({
                            "type": "Size",
                            "width": size.width,
                            "height": size.height,
                        })
                    })
                    .collect();

                document["sizes"] = json!(sizes_documents)
            }

            if let Some(tiles) = &info.tiles {
                let tile_documents: Vec<Value> = tiles
                    .iter()
                    .map(|tile| {
                        json!({
                            "type": "Tile",
                            "width": tile.width,
                            "height": tile.height,
                            "scaleFactors": tile.scale_factors
                        })
                    })
                    .collect();

                document["tiles"] = json!(tile_documents);
            }

            if let Some(rights) = &info.rights {
                document["rights"] = json!(rights);
            }

            let body = to_string_pretty(&document).expect("failed to serialize info.json");

            builder
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/ld+json")
                .body(text_body(body))
        }
    }
}
pub fn text_body<S: Into<String>>(body: S) -> HttpImageServiceBody {
    Full::<Bytes>::from(body.into())
        .map_err(|_| unreachable!())
        .boxed()
}

fn text_response<S: Into<String>>(
    status: StatusCode,
    body: S,
) -> Result<HttpImageServiceResponse, hyper::http::Error> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain")
        .body(text_body(body))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IiifRequestError {
    /// If the URI did not contain an expected element.
    UriMissingElement(&'static str),

    /// If the URI contained a text element that was not in UTF-8 (which is an RFC6570 violation).
    UriNotUtf8(&'static str),

    /// If the request contained input that could not be parsed.
    ParseError(ImageRequestParseError),
}

impl Error for IiifRequestError {}

impl Display for IiifRequestError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            IiifRequestError::UriMissingElement(element) => {
                write!(f, "Request path missing {element}.")
            }
            IiifRequestError::ParseError(error) => Display::fmt(error, f),
            IiifRequestError::UriNotUtf8(element) => {
                write!(f, "Request path {element} was not in UTF-8.")
            }
        }
    }
}

impl From<ImageRequestParseError> for IiifRequestError {
    fn from(value: ImageRequestParseError) -> Self {
        IiifRequestError::ParseError(value)
    }
}
