use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::TranscodingError;
use crate::iiif::service::ImageParameters;
use crate::image::BoxedImage;

pub fn decode_task(
    token: CancellationToken,
    mut image: BoxedImage,
    params: ImageParameters,
    output_channel: Sender<Bytes>,
) -> Result<(), TranscodingError> {
    let info = image.info();
    let mut decoder = image.open_region(params.region);

    // Process up to 32 scanlines at a time
    let buffer_capacity = info.width as usize * 1024 * 3;
    let mut buffer = BytesMut::with_capacity(buffer_capacity);

    while !token.is_cancelled() && !decoder.decode_to(&mut buffer) {
        let buf = std::mem::replace(&mut buffer, BytesMut::with_capacity(buffer_capacity));

        if output_channel.blocking_send(buf.freeze()).is_err() {
            warn!("image decoding task was cancelled prematurely");
            return Ok(());
        }
    }

    drop(buffer);

    Ok(())
}
