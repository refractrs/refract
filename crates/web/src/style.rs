//! Translate portable styles into CSS declarations for the retained DOM diff.

use refract_core::*;
use std::collections::BTreeMap;

pub(crate) fn css(e: &Element) -> BTreeMap<&'static str, String> {
    let s = &e.style;
    let mut out = BTreeMap::new();
    // Set both sets on every node: CSS variables inherit, focus styles do not.
    for (ring, color, width, offset, pointer_color, pointer_width) in [
        (s.focus_ring.unwrap_or_default(), "--refract-focus-color", "--refract-focus-width",
            "--refract-focus-offset", "--refract-edit-color", "--refract-edit-width"),
        (s.focus_within.unwrap_or(FocusRing::none()), "--refract-within-color", "--refract-within-width",
            "--refract-within-offset", "--refract-within-edit-color", "--refract-within-edit-width"),
    ] {
        out.insert(color, ring.color.css());
        out.insert(width, format!("{}px", ring.width));
        out.insert(offset, format!("{}px", ring.offset));
        out.insert(pointer_color, ring.pointer().color.css());
        out.insert(pointer_width, format!("{}px", ring.pointer().width));
    }
    let align = |a| {
        match a {
            Alignment::Start => "flex-start",
            Alignment::End => "flex-end",
            Alignment::Center => "center",
            Alignment::Stretch => "stretch",
            Alignment::SpaceBetween => "space-between",
        }
        .to_owned()
    };
    out.insert(
        "display",
        match s.layout {
            Layout::Row | Layout::Column => "flex",
            Layout::Overlay => "grid",
            _ => "block",
        }
        .into(),
    );
    if matches!(s.layout, Layout::Row | Layout::Column) {
        out.insert(
            "flex-direction",
            if s.layout == Layout::Row {
                "row"
            } else {
                "column"
            }
            .into(),
        );
        out.insert("align-items", align(s.align));
        out.insert("justify-content", align(s.justify));
        out.insert("flex-wrap", "nowrap".into());
        out.insert("gap", format!("{}px", s.gap));
    }

    // Overlay children share one grid cell, assigned by the root stylesheet.
    if s.layout == Layout::Overlay {
        out.insert("grid-template-columns", "minmax(0, 1fr)".into());
        out.insert("grid-template-rows", "minmax(0, 1fr)".into());
        let item_alignment = |value| match value {
            Alignment::Start | Alignment::SpaceBetween => "start",
            Alignment::Center => "center",
            Alignment::End => "end",
            Alignment::Stretch => "stretch",
        };
        out.insert("justify-items", item_alignment(s.justify).into());
        out.insert("align-items", item_alignment(s.align).into());
    }
    out.insert(
        "flex",
        if s.grow > 0.0 {
            format!("{} 1 0px", s.grow)
        } else {
            "0 0 auto".into()
        },
    );
    out.insert(
        "padding",
        format!(
            "{}px {}px {}px {}px",
            s.padding.top, s.padding.right, s.padding.bottom, s.padding.left
        ),
    );

    for (key, value) in [
        ("width", s.width),
        ("height", s.height),
        ("min-width", s.min_width),
        ("max-width", s.max_width),
        ("min-height", s.min_height),
        ("max-height", s.max_height),
        ("font-size", s.font_size),
        ("line-height", s.line_height),
        ("letter-spacing", s.letter_spacing),
        ("word-spacing", s.word_spacing),
    ] {
        if let Some(value) = value {
            out.insert(key, format!("{value}px"));
        }
    }
    if s.fill_width {
        out.insert("width", "100%".into());
    }
    if s.fill_height {
        out.insert("height", "100%".into());
    }

    if let Some(color) = s.foreground {
        out.insert("color", color.css());
    }
    if let Some(color) = s.background {
        out.insert("background-color", color.css());
    }
    if let Some(gradient) = &s.background_gradient {
        let stops = gradient
            .stops()
            .iter()
            .map(|stop| format!("{} {}%", stop.color.css(), stop.offset * 100.0))
            .collect::<Vec<_>>()
            .join(", ");
        let image = match gradient {
            Gradient::Linear(gradient) => {
                format!("linear-gradient({}deg in srgb, {stops})", gradient.angle)
            }
            Gradient::Radial(gradient) => format!(
                "radial-gradient(circle {}px at {}% {}% in srgb, {stops})",
                gradient.radius,
                f64::from(gradient.center.0) * 100.0,
                f64::from(gradient.center.1) * 100.0,
            ),
        };
        out.insert("background-image", image);
        // Match the desktop gradient geometry even on bordered controls.
        out.insert("background-origin", "border-box".into());
    }
    if let Some(shadow) = s.shadow {
        out.insert(
            "box-shadow",
            format!(
                "{}px {}px {}px {}",
                shadow.offset_x,
                shadow.offset_y,
                shadow.blur_radius,
                shadow.color.css()
            ),
        );
    }
    if let Some(border) = s.border {
        for (name, side) in [
            ("border-top", border.top),
            ("border-right", border.right),
            ("border-bottom", border.bottom),
            ("border-left", border.left),
        ] {
            out.insert(name, format!("{}px solid {}", side.width, side.color.css()));
        }
    }
    let r = s.radius;
    out.insert(
        "border-radius",
        format!(
            "{}px {}px {}px {}px",
            r.top_left, r.top_right, r.bottom_right, r.bottom_left
        ),
    );
    if let Some(opacity) = s.opacity {
        out.insert("opacity", opacity.to_string());
    }
    if let Some(amount) = s.blur.filter(|amount| *amount > 0.0) {
        out.insert("filter", format!("blur({amount}px)"));
    }
    if let Some(amount) = s.backdrop_blur.filter(|amount| *amount > 0.0) {
        let filter = format!("blur({amount}px)");
        out.insert("-webkit-backdrop-filter", filter.clone());
        out.insert("backdrop-filter", filter);
    }

    if let Some(weight) = s.font_weight {
        out.insert("font-weight", weight.to_string());
    }
    if let Some(italic) = s.italic {
        out.insert(
            "font-style",
            if italic { "italic" } else { "normal" }.into(),
        );
    }
    if let Some(family) = &s.font_family {
        out.insert("font-family", family.clone());
    }
    if let Some(a) = s.text_align {
        out.insert(
            "text-align",
            match a {
                Alignment::Start => "start",
                Alignment::End => "end",
                _ => "center",
            }
            .into(),
        );
    }

    if s.clipped {
        out.insert("overflow", "hidden".into());
    }
    if let Layout::Scroll(axis) = s.layout {
        out.insert(
            "overflow-x",
            if matches!(axis, Axis::Horizontal | Axis::Both) {
                "auto"
            } else {
                "hidden"
            }
            .into(),
        );
        out.insert(
            "overflow-y",
            if matches!(axis, Axis::Vertical | Axis::Both) {
                "auto"
            } else {
                "hidden"
            }
            .into(),
        );
    }
    if e.kind == ElementKind::TextInput && e.multiline {
        out.insert("resize", "none".into());
    }
    // Explicit follow policies own scroll anchoring for this viewport.
    if e.scroll
        .is_some_and(|options| options.follow != refract_core::ScrollFollow::None)
    {
        out.insert("overflow-anchor", "none".into());
    }

    if e.kind == ElementKind::Image {
        out.insert(
            "object-fit",
            match s.image_fit {
                ImageFit::Contain => "contain",
                ImageFit::Cover => "cover",
                ImageFit::Stretch => "fill",
            }
            .into(),
        );
        out.insert("object-position", "center".into());
    }
    if e.kind == ElementKind::Text {
        out.insert(
            "white-space",
            if s.truncate { "nowrap" } else { "pre-wrap" }.into(),
        );
        out.insert("overflow-wrap", "anywhere".into());
    }
    if s.truncate {
        out.insert("overflow", "hidden".into());
        out.insert("text-overflow", "ellipsis".into());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_filters_replace_and_remove_independently() {
        let Some(Description::Element(mut element, _, _)) =
            view().blur(2.5).backdrop_blur(12.0).describe()
        else {
            panic!("element")
        };
        let styled = css(&element);
        assert_eq!(styled["filter"], "blur(2.5px)");
        assert_eq!(styled["backdrop-filter"], "blur(12px)");
        assert_eq!(styled["-webkit-backdrop-filter"], styled["backdrop-filter"]);
        element.style.blur = Some(0.0);
        element.style.backdrop_blur = Some(4.0);
        let changed = css(&element);
        assert!(!changed.contains_key("filter"));
        assert_eq!(changed["backdrop-filter"], "blur(4px)");
        for value in [Some(0.0), None] {
            element.style.blur = value;
            element.style.backdrop_blur = value;
            let removed = css(&element);
            for key in ["filter", "backdrop-filter", "-webkit-backdrop-filter"] {
                assert!(!removed.contains_key(key));
            }
        }
    }

    #[test]
    fn typography_can_be_set_reset_and_removed_on_text_and_fields() {
        let Some(Description::Element(mut element, _, _)) = text("One two")
            .italic()
            .letter_spacing(-0.5)
            .word_spacing(2.0)
            .describe()
        else {
            panic!("element")
        };
        for kind in [ElementKind::Text, ElementKind::TextInput] {
            element.kind = kind;
            element.style.italic = Some(true);
            element.style.letter_spacing = Some(-0.5);
            element.style.word_spacing = Some(2.0);
            let styled = css(&element);
            assert_eq!(styled["font-style"], "italic");
            assert_eq!(styled["letter-spacing"], "-0.5px");
            assert_eq!(styled["word-spacing"], "2px");

            element.style.italic = Some(false);
            element.style.letter_spacing = Some(0.0);
            element.style.word_spacing = Some(0.0);
            let reset = css(&element);
            assert_eq!(reset["font-style"], "normal");
            assert_eq!(reset["letter-spacing"], "0px");
            assert_eq!(reset["word-spacing"], "0px");

            element.style.italic = None;
            element.style.letter_spacing = None;
            element.style.word_spacing = None;
            let removed = css(&element);
            for property in ["font-style", "letter-spacing", "word-spacing"] {
                assert!(!removed.contains_key(property));
            }
        }
    }

    #[test]
    fn gradient_kinds_replace_and_remove_without_leaving_background_properties() {
        let Some(Description::Element(mut element, _, _)) = view().describe() else {
            panic!("element")
        };
        element.style.background = Some(Color::BLACK);
        element.style.background_gradient =
            Some(LinearGradient::new(90.0, Color::BLUE, Color::WHITE).into());
        assert_eq!(
            css(&element)["background-image"],
            "linear-gradient(90deg in srgb, rgba(45,87,232,1) 0%, rgba(255,255,255,1) 100%)"
        );

        element.style.background_gradient = Some(
            RadialGradient {
                center: (-0.25, 0.75),
                radius: 80.0,
                stops: vec![
                    GradientStop::new(0.0, Color(0, 0, 0, 0)),
                    GradientStop::new(0.5, Color::BLUE),
                    GradientStop::new(0.5, Color::WHITE),
                ],
            }
            .into(),
        );
        let radial = css(&element);
        assert_eq!(radial["background-image"], "radial-gradient(circle 80px at -25% 75% in srgb, rgba(0,0,0,0) 0%, rgba(45,87,232,1) 50%, rgba(255,255,255,1) 50%)");
        assert_eq!(radial["background-origin"], "border-box");
        assert_eq!(radial["background-color"], Color::BLACK.css());

        element.style.background_gradient = None;
        let removed = css(&element);
        assert!(!removed.contains_key("background-image"));
        assert!(!removed.contains_key("background-origin"));
        assert_eq!(removed["background-color"], Color::BLACK.css());
    }

    #[test]
    fn independent_edges_and_corners_replace_and_remove_cleanly() {
        let Some(Description::Element(mut element, _, _)) = view().describe() else {
            panic!("element")
        };
        element.style.border = Some(
            Border::default()
                .top(2.0, Color::BLUE)
                .left(5.0, Color::BLACK),
        );
        element.style.radius = CornerRadii::all(12.0).top_left(0.0).bottom_right(24.0);
        let css = css(&element);
        assert_eq!(css["border-top"], "2px solid rgba(45,87,232,1)");
        assert_eq!(css["border-right"], "0px solid rgba(0,0,0,1)");
        assert_eq!(css["border-left"], "5px solid rgba(0,0,0,1)");
        assert_eq!(css["border-radius"], "0px 12px 24px 12px");
        element.style.border = None;
        element.style.radius = 4.0.into();
        let reset = super::css(&element);
        assert!(
            !reset
                .keys()
                .any(|key| key.starts_with("border-") && *key != "border-radius")
        );
        assert_eq!(reset["border-radius"], "4px 4px 4px 4px");
    }
    #[test]
    fn focus_styles_reset_and_do_not_inherit_from_ancestors() {
        let Some(Description::Element(mut element, _, _)) = view().describe() else { panic!("element") };
        element.style.focus_ring = Some(FocusRing::none());
        element.style.focus_within = Some(FocusRing::new(Color::BLACK, 3.0, 4.0));
        let custom = css(&element);
        assert_eq!(custom["--refract-focus-width"], "0px");
        assert_eq!(custom["--refract-within-width"], "3px");
        assert_eq!(custom["--refract-within-offset"], "4px");
        assert_eq!(custom["--refract-within-edit-width"], "1px");
        element.style.focus_ring = None;
        element.style.focus_within = None;
        let reset = css(&element);
        assert_eq!(reset["--refract-focus-width"], "2px");
        assert_eq!(reset["--refract-within-width"], "0px");
        assert_eq!(reset["--refract-within-edit-width"], "0px");
    }

}
