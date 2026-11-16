//! Decoded pixels: what texture decoders produce and the renderer uploads.

use crate::FormatError;

/// Bytes per RGBA8 pixel.
const PIXEL_LEN: usize = 4;

/// Largest image we allocate, in pixels (8192 × 8192). Decoders check their
/// input against this before allocating.
pub const MAX_PIXELS: u64 = 8192 * 8192;

/// An image of RGBA8 pixels, row by row from the top left, no padding.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaImage {
    /// A transparent black image.
    pub fn new(width: u32, height: u32) -> Result<Self, FormatError> {
        let len = Self::byte_len(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![0; len],
        })
    }

    /// An image from RGBA8 bytes; `None` if there are not exactly
    /// `width × height × 4` of them or the image is over [`MAX_PIXELS`].
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let len = Self::byte_len(width, height).ok()?;
        (pixels.len() == len).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    /// An image of one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self, FormatError> {
        let len = Self::byte_len(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: rgba.repeat(len / PIXEL_LEN),
        })
    }

    fn byte_len(width: u32, height: u32) -> Result<usize, FormatError> {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > MAX_PIXELS {
            return Err(FormatError::TooLarge {
                what: "image",
                size: pixels * PIXEL_LEN as u64,
                limit: MAX_PIXELS * PIXEL_LEN as u64,
            });
        }
        Ok(pixels as usize * PIXEL_LEN)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA8 bytes, row by row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Takes the RGBA8 bytes.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height)
            .then(|| (y as usize * self.width as usize + x as usize) * PIXEL_LEN)
    }

    /// The pixel at (`x`, `y`), or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let at = self.offset(x, y)?;
        let p = self.pixels.get(at..at + PIXEL_LEN)?;
        p.try_into().ok()
    }

    /// Sets the pixel at (`x`, `y`); does nothing outside the image.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if let Some(at) = self.offset(x, y)
            && let Some(p) = self.pixels.get_mut(at..at + PIXEL_LEN)
        {
            p.copy_from_slice(&rgba);
        }
    }

    /// Copies `other` into this image with its top left corner at (`x`, `y`),
    /// clipped to this image.
    pub fn blit(&mut self, other: &RgbaImage, x: u32, y: u32) {
        for row in 0..other.height {
            for col in 0..other.width {
                if let Some(p) = other.pixel(col, row) {
                    self.set_pixel(x.saturating_add(col), y.saturating_add(row), p);
                }
            }
        }
    }

    /// The `width` × `height` part from (`x`, `y`), cut to the image;
    /// `None` if nothing of it is inside.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Option<RgbaImage> {
        let width = width.min(self.width.saturating_sub(x));
        let height = height.min(self.height.saturating_sub(y));
        if width == 0 || height == 0 {
            return None;
        }
        let row_len = width as usize * PIXEL_LEN;
        let mut pixels = Vec::with_capacity(row_len * height as usize);
        for row in y..y + height {
            let at = self.offset(x, row)?;
            pixels.extend_from_slice(self.pixels.get(at..at + row_len)?);
        }
        RgbaImage::from_pixels(width, height, pixels)
    }
}

/// Prints the size only, not the pixels.
impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}×{})", self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_are_cut_to_the_image() {
        let mut image = RgbaImage::new(4, 3).unwrap();
        image.set_pixel(1, 1, [1, 2, 3, 4]);
        image.set_pixel(3, 2, [5, 6, 7, 8]);
        let part = image.crop(1, 1, 2, 2).unwrap();
        assert_eq!((part.width(), part.height()), (2, 2));
        assert_eq!(part.pixel(0, 0), Some([1, 2, 3, 4]));
        let corner = image.crop(3, 2, 10, 10).unwrap();
        assert_eq!((corner.width(), corner.height()), (1, 1));
        assert_eq!(corner.pixel(0, 0), Some([5, 6, 7, 8]));
        assert_eq!(image.crop(4, 0, 1, 1), None);
        assert_eq!(image.crop(0, 0, 0, 1), None);
    }
}
