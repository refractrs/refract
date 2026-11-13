//! Portable vector geometry, views, and drawing commands.
mod canvas;
mod shape;
pub use canvas::*;
pub use shape::*;

mod geometry;
mod graphics;
pub use geometry::*;
pub use graphics::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Affine2d, Color};

    #[test]
    fn paths_share_storage_until_modified() {
        let original = Path::rectangle(Rect::new(2.0, 3.0, 8.0, 9.0));
        let mut changed = original.clone();
        assert_eq!(original.elements().as_ptr(), changed.elements().as_ptr());
        changed.line_to((40.0, 50.0));
        assert_eq!(original.bounds(), Some(Rect::new(2.0, 3.0, 8.0, 9.0)));
        assert_eq!(changed.bounds(), Some(Rect::new(2.0, 3.0, 38.0, 47.0)));
    }

    #[test]
    fn ellipse_and_transformed_effect_bounds_include_coverage() {
        let path = Path::ellipse(Rect::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(path.elements().len(), 6);
        let mut g = GraphicsContext::new();
        g.transformed(Affine2d([2.0, 0.0, 0.0, 2.0, 100.0, 50.0]), |g| {
            g.layer(Layer::new().blur(2.0), |g| g.fill(&path, Color::WHITE));
        });
        let commands = g.finish().unwrap();
        let bounds = drawing_bounds(&commands).unwrap();
        assert!(bounds.x <= 88.0 && bounds.y <= 38.0);
        assert!(bounds.x + bounds.width >= 152.0);
        assert!(bounds.y + bounds.height >= 82.0);
    }

    #[test]
    fn invalid_nested_drawings_are_rejected() {
        let path = Path::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
        let mut g = GraphicsContext::new();
        g.layer(Layer::new(), |g| {
            g.stroke(
                &path,
                Color::BLACK,
                StrokeStyle::new(1.0).dash(vec![0.0, 0.0]),
            )
        });
        assert!(g.finish().is_err());
        let mut g = GraphicsContext::new();
        g.layer(
            Layer::new().mask(Mask::new(
                path.clone(),
                Paint::radial(Point::ZERO, 0.0, vec![]),
            )),
            |g| g.fill(&path, Color::WHITE),
        );
        assert!(g.finish().is_err());
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::Color;
    #[test]
    fn transparent_canvas_colors_interpolate_without_dark_fringes() {
        let target = Color(255, 0, 0, 255);
        let transparent = Color(0, 0, 0, 0);
        let midpoint = transparent
            .components()
            .iter()
            .zip(target.components())
            .map(|(a, b)| (a + b) / 2.0)
            .collect::<Vec<_>>();
        assert_eq!(target.with_components(&midpoint), Color(255, 0, 0, 128));
    }
}
