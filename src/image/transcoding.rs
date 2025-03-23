use std::error::Error;
use std::fmt::Display;
use std::io::Write;
use std::pin::pin;
use std::task::Poll;

use bytes::{BufMut, Bytes, BytesMut};
use decode::decode_task;
use encode::encode_task;
use futures::stream::Fuse;
use futures::{Stream, StreamExt};
use gcd::Gcd;
use mediatype::MediaTypeBuf;
use mediatype::names::{IMAGE, JPEG};
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span};

use super::{AbsoluteRegion, BoxedImage, ImageStream};
use crate::iiif::service::ImageParameters;
use crate::iiif::{Region, Scale};

pub mod decode;
pub mod encode;

/// Coordinates the processing of an image according to IIIF parameters.
///
/// This struct orchestrates the decoding, transformation, and encoding of image data
/// in an efficient, streaming manner. It's designed to handle large images by
/// processing them in chunks rather than loading the entire image into memory.
///
/// The pipeline spawns separate tasks for decoding and encoding, connected by channels:
/// 1. A decoder task extracts and processes the requested region of the source image
/// 2. An encoder task compresses the image data to the target format
/// 3. The resulting stream yields compressed image data as it becomes available
///
/// # Example
///
/// ```
/// use laya::iiif::service::ImageParameters;
/// use laya::image::transcoding::TranscodingPipeline;
///
/// let pipeline = TranscodingPipeline {
///     image: source_image,
///     params: image_parameters,
/// };
///
/// // Run the pipeline and get a stream of encoded image data
/// let image_stream = pipeline.run();
/// ```
pub struct TranscodingPipeline {
    pub image: BoxedImage,
    pub params: ImageParameters,
}

#[derive(Debug)]
pub enum TranscodingError {
    Generic(String),
    Io(std::io::Error),
    Unknown,
}

impl Error for TranscodingError {}

impl From<std::io::Error> for TranscodingError {
    fn from(value: std::io::Error) -> Self {
        TranscodingError::Io(value)
    }
}

impl Display for TranscodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscodingError::Generic(message) => write!(f, "{message}"),
            TranscodingError::Io(err) => write!(f, "io error: {err}"),
            TranscodingError::Unknown => write!(f, "unknown error"),
        }
    }
}

/// A type that implements [Write] by buffering writes
/// and sending them to a channel when the buffer is full or when flushed.
///
/// `SenderWriter` maintains an internal buffer and a sender channel. When data is written,
/// it's first added to the buffer. Once the buffer reaches a certain size (4096 bytes),
/// or when explicitly flushed, the buffered data is sent through the channel.
pub struct SenderWriter {
    buffer: BytesMut,
    sender: Sender<Bytes>,
}

impl SenderWriter {
    pub fn new(sender: Sender<Bytes>) -> SenderWriter {
        Self { buffer: BytesMut::with_capacity(4096 * 16), sender }
    }
}

impl Write for SenderWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.put(buf);

        if self.buffer.len() >= 4096 * 16 {
            self.flush()?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.sender
            .blocking_send(
                std::mem::replace(&mut self.buffer, BytesMut::with_capacity(4096 * 16)).freeze(),
            )
            .map_err(std::io::Error::other)
    }
}

impl TranscodingPipeline {
    pub fn run(self) -> ImageStream {
        let Self { mut image, params } = self;

        let info = image.info();
        let token = CancellationToken::new();
        let mut task_set = JoinSet::new();

        let absolute_region = match params.region {
            Region::Absolute { x, y, width, height } => AbsoluteRegion { x, y, width, height },
            Region::Full => AbsoluteRegion { x: 0, y: 0, width: info.width, height: info.height },
            _ => todo!(),
        };

        let dims_gcd = info.width.gcd(info.height);
        let ratio_w = info.width / dims_gcd;
        let ratio_h = info.height / dims_gcd;

        let size = match params.size.scale() {
            Scale::Max => (absolute_region.width, absolute_region.height),
            Scale::Fixed { width: scaled_width, height: scaled_height } => {
                (scaled_width.get(), scaled_height.get())
            }
            Scale::FixedWidth(scaled_width) => {
                (scaled_width.get(), (scaled_width.get() * ratio_h) / ratio_w)
            }
            Scale::FixedHeight(scaled_height) => {
                ((scaled_height.get() * ratio_w) / ratio_h, scaled_height.get())
            }
            Scale::Percentage(pct) => {
                let scaled_x = absolute_region.width as f32 / 100.0 * pct;
                let scaled_y = absolute_region.height as f32 / 100.0 * pct;

                (scaled_x.ceil() as u32, scaled_y.ceil() as u32)
            }
            Scale::AspectPreserving { width, height } => {
                let scale_w = height.get() as f64 / absolute_region.width as f64;
                let scale_h = width.get() as f64 / absolute_region.height as f64;
                let scale = scale_w.min(scale_h);

                let new_w = (absolute_region.width as f64 * scale).ceil() as u32;
                let new_h = (absolute_region.height as f64 * scale).ceil() as u32;

                (new_w, new_h)
            }
        };

        info!("Calculated dimensions ({size:?}) for scale params: {:?}", params.size.scale());

        let decoder_token = token.clone();
        let decoder_span = info_span!("image_decoder", decoder = "kakadu");
        let (decoded_tx, decoded_rx) = mpsc::channel(4);

        task_set.spawn_blocking(move || -> Result<(), TranscodingError> {
            decoder_span
                .in_scope(|| decode_task(decoder_token, image, absolute_region, size, decoded_tx))
        });

        let encoder_token = token.clone();
        let encoder_span = info_span!("image_encoder", encoder = "mozjpeg");
        let (encoded_tx, encoded_rx) = mpsc::channel(4);

        task_set.spawn_blocking(move || -> Result<(), TranscodingError> {
            encoder_span.in_scope(|| encode_task(encoder_token, size, decoded_rx, encoded_tx, info))
        });

        ImageStream {
            media_type: MediaTypeBuf::new(IMAGE, JPEG),
            data: Box::new(TranscodedStream {
                task_set,
                token,
                receiver: ReceiverStream::new(encoded_rx).fuse(),
            }),
        }
    }
}

/// A stream that yields encoded image data from a transcoding pipeline.
///
/// `TranscodedStream` coordinates between the image decoder and encoder tasks,
/// managing their lifecycle and propagating errors appropriately. It implements
/// the [Stream] trait to produce a sequence of [Bytes] containing the encoded image.
///
/// The stream completes when:
/// - All transcoding tasks complete successfully and all encoded data is yielded
/// - An error occurs in any task, causing cancellation of the pipeline
/// - The associated cancellation token is triggered externally
pub struct TranscodedStream {
    task_set: JoinSet<Result<(), TranscodingError>>,
    token: CancellationToken,
    receiver: Fuse<ReceiverStream<Bytes>>,
}

impl Stream for TranscodedStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let task_set = &mut this.task_set;

        while let Poll::Ready(Some(res)) = task_set.poll_join_next(cx) {
            match res {
                Ok(Err(e)) => {
                    error!("An error occurred during transcoding: {e}");
                    this.token.cancel();

                    return Poll::Ready(Some(Err(std::io::Error::other(e))));
                }
                Err(e) => {
                    error!("Transcoding task panicked: {e}");
                    this.token.cancel();

                    return Poll::Ready(Some(Err(std::io::Error::other(e))));
                }
                _ => {} // Task completed successfully
            }
        }

        let inner = pin!(&mut this.receiver);

        match inner.poll_next(cx) {
            Poll::Ready(Some(data)) => return Poll::Ready(Some(Ok(data))),
            Poll::Ready(None) if this.task_set.is_empty() => return Poll::Ready(None),
            _ => {}
        }

        Poll::Pending
    }
}
