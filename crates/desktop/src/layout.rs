//! Translation of Refract's box model into Taffy styles.

use refract_core::{Alignment, Element, ElementKind, Layout};
use taffy::prelude::*;

pub(crate) fn style(element: &Element) -> Style {
    let source = &element.style;
    let border = source.border.unwrap_or_default();
    let mut result = Style {
        display: match source.layout {
            Layout::Row | Layout::Column => Display::Flex,
            Layout::Overlay => Display::Grid,
            _ => Display::Block,
        },
        flex_direction: if source.layout == Layout::Row {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        },
        flex_grow: source.grow,
        flex_shrink: if source.grow > 0.0 { 1.0 } else { 0.0 },
        flex_basis: if source.grow > 0.0 {
            Dimension::length(0.0)
        } else {
            Dimension::auto()
        },
        align_items: align(source.align),
        justify_content: justify(source.justify),
        gap: Size {
            width: length(source.gap),
            height: length(source.gap),
        },
        padding: Rect {
            left: length(source.padding.left),
            right: length(source.padding.right),
            top: length(source.padding.top),
            bottom: length(source.padding.bottom),
        },
        border: Rect {
            left: length(border.left.width),
            right: length(border.right.width),
            top: length(border.top.width),
            bottom: length(border.bottom.width),
        },
        size: Size {
            width: dimension(source.width, source.fill_width),
            height: dimension(source.height, source.fill_height),
        },
        min_size: Size {
            width: source
                .min_width
                .map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length),
            height: source
                .min_height
                .map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length),
        },
        max_size: Size {
            width: source
                .max_width
                .map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length),
            height: source
                .max_height
                .map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length),
        },
        ..Style::default()
    };
    if element.kind == ElementKind::TextInput {
        // Editable controls have a stable intrinsic size; contents scroll inside.
        if source.width.is_none() && !source.fill_width {
            result.size.width = Dimension::length(200.0);
        }
        if source.height.is_none() && !source.fill_height {
            result.size.height = Dimension::length(if element.multiline {
                96.0
            } else {
                36.0
            });
        }
    }
    if source.layout == Layout::Overlay {
        result.grid_template_columns = vec![fr(1.0)];
        result.grid_template_rows = vec![fr(1.0)];
    }
    // All children of an overlay use its single grid cell. These placement values
    // are ignored by flex/block parents, so no parent-dependent restyling is needed.
    result.grid_row = Line {
        start: line(1),
        end: line(2),
    };
    result.grid_column = Line {
        start: line(1),
        end: line(2),
    };
    if source.clipped || matches!(source.layout, Layout::Scroll(_)) {
        result.overflow = taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Hidden,
        };
    }
    result
}

fn length(value: f32) -> LengthPercentage {
    LengthPercentage::length(value)
}
fn dimension(value: Option<f32>, fill: bool) -> Dimension {
    if fill {
        Dimension::percent(1.0)
    } else {
        value.map_or(Dimension::auto(), Dimension::length)
    }
}
fn align(value: Alignment) -> AlignItems {
    match value {
        Alignment::Start | Alignment::SpaceBetween => AlignItems::START,
        Alignment::Center => AlignItems::CENTER,
        Alignment::End => AlignItems::END,
        Alignment::Stretch => AlignItems::STRETCH,
    }
}
fn justify(value: Alignment) -> JustifyContent {
    match value {
        Alignment::Start => JustifyContent::START,
        Alignment::End => JustifyContent::END,
        Alignment::Center => JustifyContent::CENTER,
        Alignment::Stretch => JustifyContent::STRETCH,
        Alignment::SpaceBetween => JustifyContent::SPACE_BETWEEN,
    }
}
