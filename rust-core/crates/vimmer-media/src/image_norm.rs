use std::path::Path;

use image::{
    imageops::{self, FilterType},
    RgbImage,
};
use vimmer_dsl::Resolution;

use crate::MediaError;

const BACKGROUND_BLUR_SIGMA: f32 = 40.0;
const BACKGROUND_BRIGHTNESS: f32 = 0.45;

/// Loads `src`, fits it into `target` and writes a PNG to `dst`.
pub fn normalize_image(src: &Path, dst: &Path, target: Resolution) -> Result<(), MediaError> {
    let source = image::open(src)?.to_rgb8();
    letterbox(&source, target).save(dst)?;
    Ok(())
}

/// Places `src` centered on a `target`-sized canvas. The background is the
/// same image cover-filled, blurred and darkened, so mismatched aspect ratios
/// get a soft padding instead of flat bars.
pub fn letterbox(src: &RgbImage, target: Resolution) -> RgbImage {
    let (tw, th) = (target.width, target.height);

    let mut background = imageops::blur(&cover(src, tw, th), BACKGROUND_BLUR_SIGMA);
    for pixel in background.pixels_mut() {
        for channel in pixel.0.iter_mut() {
            *channel = (f32::from(*channel) * BACKGROUND_BRIGHTNESS) as u8;
        }
    }

    let foreground = contain(src, tw, th);
    let x = i64::from((tw - foreground.width()) / 2);
    let y = i64::from((th - foreground.height()) / 2);
    imageops::overlay(&mut background, &foreground, x, y);
    background
}

/// Scales to fully cover `tw` x `th`, then crops the centre.
fn cover(src: &RgbImage, tw: u32, th: u32) -> RgbImage {
    let scale = f64::max(
        f64::from(tw) / f64::from(src.width()),
        f64::from(th) / f64::from(src.height()),
    );
    let nw = ((f64::from(src.width()) * scale).ceil() as u32).max(tw);
    let nh = ((f64::from(src.height()) * scale).ceil() as u32).max(th);
    let resized = imageops::resize(src, nw, nh, FilterType::Lanczos3);
    let x = (nw - tw) / 2;
    let y = (nh - th) / 2;
    imageops::crop_imm(&resized, x, y, tw, th).to_image()
}

/// Scales to fit inside `tw` x `th` without cropping.
fn contain(src: &RgbImage, tw: u32, th: u32) -> RgbImage {
    let scale = f64::min(
        f64::from(tw) / f64::from(src.width()),
        f64::from(th) / f64::from(src.height()),
    );
    let nw = ((f64::from(src.width()) * scale).round() as u32).clamp(1, tw);
    let nh = ((f64::from(src.height()) * scale).round() as u32).clamp(1, th);
    imageops::resize(src, nw, nh, FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn solid(w: u32, h: u32, color: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(color))
    }

    #[test]
    fn output_matches_target_size() {
        let out = letterbox(
            &solid(400, 300, [200, 0, 0]),
            Resolution {
                width: 1920,
                height: 1080,
            },
        );
        assert_eq!((out.width(), out.height()), (1920, 1080));
    }

    #[test]
    fn portrait_target_from_landscape_source() {
        let out = letterbox(
            &solid(1600, 900, [0, 200, 0]),
            Resolution {
                width: 1080,
                height: 1920,
            },
        );
        assert_eq!((out.width(), out.height()), (1080, 1920));
    }

    #[test]
    fn centre_keeps_source_colour_and_edges_are_darkened_blur() {
        let out = letterbox(
            &solid(400, 300, [200, 100, 50]),
            Resolution {
                width: 1920,
                height: 1080,
            },
        );
        let centre = out.get_pixel(960, 540).0;
        assert!(centre[0] > 180, "centre should keep source red: {centre:?}");
        // Top-left falls in the padded background, which is darkened.
        let corner = out.get_pixel(5, 5).0;
        assert!(corner[0] < 120, "background should be darkened: {corner:?}");
    }
}
