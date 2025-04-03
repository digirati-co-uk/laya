use std::num::NonZero;
use std::ops::{Deref, DerefMut};

use bytes::{Bytes, BytesMut};
use futures::Stream;
use mediatype::MediaTypeBuf;

pub mod info;
pub mod reader;
pub mod transcoding;

use info::ImageInfo;
pub use reader::ImageReader;

use crate::iiif::{Dimension, Region, Scale};

pub type Dimensions = (Dimension, Dimension);

#[derive(Debug)]
pub struct Dimensions2 {
    width: Dimension,
    height: Dimension,
}

pub type Rect = AbsoluteRegion;

#[derive(Debug)]
pub struct Crop {
    /// The region of this crop within the high-resolution canvas of the source image.
    hires_region: Rect,

    /// The output dimensions of the cropped high-resolution region.
    scale: Dimensions,
}

impl Dimensions2 {
    #[tracing::instrument]
    pub fn crop_and_scale(self, region: Region, scale: Scale) -> Option<Crop> {
        let Self { width, height } = self;
        let rect = match region {
            Region::Absolute { x, y, width, height } => AbsoluteRegion { x, y, width, height },
            Region::Full => AbsoluteRegion { x: 0, y: 0, width, height },
            _ => todo!(),
        };

        let (target_width, target_height) = match scale {
            Scale::Max => (width, height),
            Scale::Fixed { width: scaled_width, height: scaled_height } => (scaled_width.get(), scaled_height.get()),
            Scale::FixedWidth(scaled_width) => (scaled_width.get(), (scaled_width.get() * height) / width),
            Scale::FixedHeight(scaled_height) => ((scaled_height.get() * width) / height, scaled_height.get()),
            Scale::Percentage(pct) => {
                let scaled_x = rect.width as f32 / 100.0 * pct;
                let scaled_y = rect.height as f32 / 100.0 * pct;

                (NonZero::new(scaled_x as u32)?.get(), NonZero::new(scaled_y as u32)?.get())
            }
            Scale::AspectPreserving { width: target_width, height: target_height } => {
                let image_ar = width as f64 / height as f64;
                let target_width = target_width.get().min(rect.width);
                let target_height = target_height.get().min(rect.height);
                let scaled_h = (target_width as f64 * image_ar).round() as u32;

                if scaled_h <= target_height {
                    (target_width, NonZero::new(scaled_h)?.get())
                } else {
                    let scaled_w = (target_height as f64 / image_ar).round() as u32;
                    (NonZero::new(scaled_w)?.get(), target_height)
                }
            }
        };

        Some(Crop { hires_region: rect, scale: (target_width, target_height) })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AbsoluteRegion {
    x: Dimension,
    y: Dimension,
    width: Dimension,
    height: Dimension,
}

/// An asynchronous sequential stream of encoded image data and the associated
/// [mediatype::MediaType]
pub struct ImageStream {
    pub media_type: MediaTypeBuf,
    pub data: Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Sync + Unpin>,
}

pub trait ImageDecoder {
    fn output_size(&self) -> Dimensions;
    fn decode_to(&mut self, buffer: &mut BytesMut) -> bool;
}

pub trait Image {
    fn boxed(self) -> BoxedImage
    where
        Self: Sized + Send + 'static,
    {
        BoxedImage(Box::new(self))
    }

    fn info(&mut self) -> ImageInfo;

    fn open_region(&mut self, region: AbsoluteRegion, scaled_to: Dimensions) -> Box<dyn ImageDecoder>;
}

pub struct BoxedImage(Box<dyn Image + Send>);

impl Deref for BoxedImage {
    type Target = Box<dyn Image + Send>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for BoxedImage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Image for BoxedImage {
    fn info(&mut self) -> ImageInfo {
        self.0.info()
    }

    fn open_region(&mut self, region: AbsoluteRegion, scaled_to: (Dimension, Dimension)) -> Box<dyn ImageDecoder> {
        self.0.open_region(region, scaled_to)
    }
}
