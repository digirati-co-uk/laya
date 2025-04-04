use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::TranscodingError;
use crate::image::{AbsoluteRegion, BoxedImage, Dimensions};

pub fn decode_task(
    token: CancellationToken,
    mut image: BoxedImage,
    absolute_region: AbsoluteRegion,
    size: Dimensions,
    output_channel: Sender<Bytes>,
) -> Result<(), TranscodingError> {
    let info = image.info();
    let mut decoder = image.open_region(absolute_region, size);
    let scanlines = info
        .tiles
        .iter()
        .flatten()
        .filter(|tile| (tile.width < info.width))
        .map(|tile| tile.height)
        .max()
        .flatten()
        .unwrap_or(512);

    let buffer_capacity = info.width as usize * scanlines as usize * 3;
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
