//! Outer shadows with an interior knockout, including transparent backgrounds.
//! Uniform corners use Vello's analytic blur. Asymmetric corners use a bounded,
//! cached alpha mask so all four corners contribute to the same Gaussian blur.

use super::{Rect, vello_geometry::RoundedBox};
use refract_core::{BoxShadow, CornerRadii};
use std::{collections::VecDeque, sync::Arc};
use vello::{
    Scene,
    kurbo::{Affine, Shape},
    peniko::{Blob, Color, Fill, ImageAlphaType, ImageData, ImageFormat},
};

#[derive(Default)]
pub(super) struct ShadowCache(VecDeque<([u32; 9], ImageData, f64)>);

impl ShadowCache {
    fn mask(
        &mut self,
        rect: Rect,
        radii: CornerRadii,
        shadow: BoxShadow,
        scale: f64,
    ) -> (ImageData, f64) {
        let key = [
            rect.width.to_bits(),
            rect.height.to_bits(),
            radii.top_left.to_bits(),
            radii.top_right.to_bits(),
            radii.bottom_right.to_bits(),
            radii.bottom_left.to_bits(),
            shadow.blur_radius.to_bits(),
            u32::from_le_bytes([
                shadow.color.0,
                shadow.color.1,
                shadow.color.2,
                shadow.color.3,
            ]),
            (scale as f32).to_bits(),
        ];
        if let Some(index) = self.0.iter().position(|entry| entry.0 == key) {
            let entry = self.0.remove(index).unwrap();
            let result = (entry.1.clone(), entry.2);
            self.0.push_back(entry);
            return result;
        }
        let (data, resolution) = rasterize(rect, radii, shadow, scale);
        // At most eight 1024² RGBA masks; moving a shadow does not regenerate it.
        if self.0.len() == 8 {
            self.0.pop_front();
        }
        self.0.push_back((key, data.clone(), resolution));
        (data, resolution)
    }
}

fn rasterize(rect: Rect, radii: CornerRadii, shadow: BoxShadow, scale: f64) -> (ImageData, f64) {
    let sigma = f64::from(shadow.blur_radius) / 2.0;
    let pad = 3.0 * sigma;
    let width = f64::from(rect.width) + 2.0 * pad;
    let height = f64::from(rect.height) + 2.0 * pad;
    let resolution = scale.max(0.01).min(1024.0 / width.max(height));
    let w = (width * resolution).ceil().max(1.0) as u32;
    let h = (height * resolution).ceil().max(1.0) as u32;
    let shape = RoundedBox::new(
        Rect {
            x: 0.0,
            y: 0.0,
            ..rect
        },
        radii,
    );
    let mask = image::GrayImage::from_fn(w, h, |x, y| {
        let mut covered = 0;
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            if shape.contains(
                (f64::from(x) + dx) / resolution - pad,
                (f64::from(y) + dy) / resolution - pad,
            ) {
                covered += 1;
            }
        }
        image::Luma([(covered * 255 / 4) as u8])
    });
    let mask = image::imageops::fast_blur(&mask, (sigma * resolution) as f32);
    let c = shadow.color;
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for alpha in mask.into_raw() {
        pixels.extend([
            c.0,
            c.1,
            c.2,
            (u16::from(alpha) * u16::from(c.3) / 255) as u8,
        ]);
    }
    (
        ImageData {
            data: Blob::new(Arc::new(pixels)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: w,
            height: h,
        },
        resolution,
    )
}

pub(super) fn draw(
    scene: &mut Scene,
    cache: &mut ShadowCache,
    scale: f64,
    rect: Rect,
    radius: CornerRadii,
    shadow: BoxShadow,
) {
    if shadow.color.3 == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let transform = Affine::scale(scale);
    let border = RoundedBox::new(rect, radius);
    let mut caster = border;
    caster.bounds = caster.bounds
        + vello::kurbo::Vec2::new(f64::from(shadow.offset_x), f64::from(shadow.offset_y));
    let sigma = f64::from(shadow.blur_radius) / 2.0;
    let extent = caster
        .bounds
        .inflate(3.0 * sigma, 3.0 * sigma)
        .union(border.bounds);
    let mut knockout = extent.to_path(0.1);
    knockout.extend(border.path().elements().iter().copied());
    let color = Color::from_rgba8(
        shadow.color.0,
        shadow.color.1,
        shadow.color.2,
        shadow.color.3,
    );
    scene.push_clip_layer(Fill::EvenOdd, transform, &knockout);
    if sigma == 0.0 {
        scene.fill(Fill::NonZero, transform, color, None, &caster.path());
    } else if border.corners.iter().all(|r| *r == border.corners[0]) {
        scene.draw_blurred_rounded_rect(
            transform,
            caster.bounds,
            color,
            border.corners[0].0,
            sigma,
        );
    } else {
        let (data, resolution) = cache.mask(rect, radius, shadow, scale);
        scene.draw_image(
            &data,
            transform
                * Affine::translate((
                    caster.bounds.x0 - 3.0 * sigma,
                    caster.bounds.y0 - 3.0 * sigma,
                ))
                * Affine::scale(1.0 / resolution),
        );
    }
    scene.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::Color;

    #[test]
    fn asymmetric_shadow_mask_preserves_corner_shape_and_reuses_pixels_when_moved() {
        let rect = Rect {
            width: 80.0,
            height: 80.0,
            ..Rect::default()
        };
        let radii = CornerRadii::default().top_left(40.0);
        let shadow = BoxShadow::new(-5.0, 4.0, 4.0, Color::BLACK);
        let mut cache = ShadowCache::default();
        let (mask, resolution) = cache.mask(rect, radii, shadow, 1.0);
        assert_eq!(resolution, 1.0);
        let alpha = |x: usize, y: usize| mask.data.as_ref()[(y * mask.width as usize + x) * 4 + 3];
        assert!(
            alpha(10, 10) < alpha(81, 10),
            "only the top-left corner is rounded"
        );
        let moved = Rect {
            x: 100.0,
            y: 30.0,
            ..rect
        };
        let (reused, _) = cache.mask(
            moved,
            radii,
            BoxShadow {
                offset_x: 10.0,
                ..shadow
            },
            1.0,
        );
        assert_eq!(mask.data.id(), reused.data.id());
        let (large, _) = rasterize(
            Rect {
                width: 10_000.0,
                ..rect
            },
            radii,
            shadow,
            2.0,
        );
        assert!(large.width <= 1024 && large.height <= 1024);
    }

    #[test]
    fn knockout_excludes_original_asymmetric_box_with_signed_offsets() {
        let rect = Rect {
            x: 20.0,
            y: 30.0,
            width: 100.0,
            height: 60.0,
        };
        let border = RoundedBox::new(rect, CornerRadii::default().top_left(30.0));
        let mut knockout = border.bounds.inflate(30.0, 30.0).to_path(0.1);
        knockout.extend(border.path().elements().iter().copied());
        assert_eq!(knockout.winding((70.0, 60.0).into()) % 2, 0);
        assert_ne!(knockout.winding((21.0, 31.0).into()) % 2, 0);
        assert_eq!(knockout.winding((119.0, 31.0).into()) % 2, 0);
    }
}
