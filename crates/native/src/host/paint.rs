//! Layout invalidation and painting. A frame references cached glyph runs; it
//! never evaluates views or reads application state.

use super::Host;
use crate::image::ImageState;
use crate::{
    renderer::{DrawCommand, Frame, Point, Rect},
    Result,
};
use refract_core::{Color, ElementId, ElementKind, ImageFit, Insets, Layout};
use taffy::{
    compute::compute_leaf_layout,
    prelude::{AvailableSpace, Dimension, Size},
};

impl Host {
    pub fn frame(
        &mut self,
        width: f32,
        height: f32,
        scale_factor: f64,
        background: Color,
    ) -> Result<Frame> {
        self.frame_at(
            width,
            height,
            scale_factor,
            background,
            std::time::Instant::now(),
        )
    }

    pub(super) fn frame_at(
        &mut self,
        width: f32,
        height: f32,
        scale_factor: f64,
        background: Color,
        now: std::time::Instant,
    ) -> Result<Frame> {
        let resized = self.viewport != (width, height);
        if resized {
            self.viewport = (width, height);
            let mut style = self.tree.style(self.root)?.clone();
            style.size = Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            };
            self.tree.set_style(self.root, style)?;
            self.needs_layout = true;
        }
        if self.needs_layout {
            self.tree.compute_layout_with_measure(
                self.root,
                Size {
                    width: AvailableSpace::Definite(width),
                    height: AvailableSpace::Definite(height),
                },
                |inputs, _, context, style| {
                    compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, available| {
                            let Some(context) = context else {
                                return Size::ZERO;
                            };
                            let mut context = context.borrow_mut();
                            if let Some((width, height)) = context.image_size {
                                return image_measure(width, height, known, available);
                            }
                            let align = context.align;
                            let Some(block) = context.block.as_mut() else {
                                return Size::ZERO;
                            };
                            // A constrained text box occupies its available width
                            // between min-/max-content bounds. The longest *wrapped*
                            // line is only glyph coverage, not the box's width: using
                            // it makes centered ancestors jump at word-wrap thresholds.
                            let (min, max) = block.content_widths();
                            let width = known.width.unwrap_or(match available.width {
                                AvailableSpace::Definite(width) => width.min(max).max(min),
                                AvailableSpace::MinContent => min,
                                AvailableSpace::MaxContent => max,
                            });
                            let (_, height) = block.measure(Some(width), align);
                            Size {
                                width,
                                height: known.height.unwrap_or(height),
                            }
                        },
                    )
                },
            )?;
            self.stats.layout_passes += 1;
            self.needs_layout = false;
        }
        self.resolve_scroll_layout()?;
        let mut frame = Frame {
            commands: vec![],
            background,
            scale_factor,
        };
        self.paint_order.clear();
        self.scrolling.begin_frame(now, resized);
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        for id in self.roots.clone() {
            self.paint(id, Point::default(), clip, &mut frame)?;
        }
        self.scrolling.end_frame();
        self.paint_menu(&mut frame);
        Ok(frame)
    }

    fn paint(
        &mut self,
        id: ElementId,
        origin: Point,
        inherited_clip: Rect,
        frame: &mut Frame,
    ) -> Result<()> {
        let focus_within = self.has_focus_within(id);
        let entry = self.entries.get_mut(&id).unwrap();
        let layout = self.tree.layout(entry.node)?;
        let rect = Rect {
            x: origin.x + layout.location.x,
            y: origin.y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        entry.rect = rect;
        let s = &entry.element.style;
        let clipped = s.clipped || matches!(s.layout, Layout::Scroll(_));
        entry.clip = if clipped {
            inherited_clip.intersect(rect)
        } else {
            inherited_clip
        };
        let clip = entry.clip;
        self.paint_order.push(id);
        // Opacity includes the shadow, but the element's own content clip does
        // not cut it off. Ancestor clips continue to constrain the whole group.
        let has_effects =
            s.blur.is_some_and(|v| v > 0.0) || s.backdrop_blur.is_some_and(|v| v > 0.0);
        if has_effects {
            frame.commands.push(DrawCommand::PushEffects {
                rect,
                radius: s.radius,
                blur: s.blur.unwrap_or(0.0),
                backdrop_blur: s.backdrop_blur.unwrap_or(0.0),
                opacity: s.opacity.unwrap_or(1.0),
            });
        } else if let Some(opacity) = s.opacity {
            frame.commands.push(DrawCommand::PushLayer {
                clip: inherited_clip,
                radius: 0.0.into(),
                opacity,
            });
        }
        if let Some(shadow) = s.shadow {
            frame.commands.push(DrawCommand::BoxShadow {
                rect,
                radius: s.radius,
                shadow,
            });
        }
        if clipped {
            frame.commands.push(DrawCommand::PushLayer {
                clip: rect,
                radius: s.radius,
                opacity: 1.0,
            });
        }
        if let Some(color) = s.background {
            frame.commands.push(DrawCommand::Fill {
                rect,
                radius: s.radius,
                color,
            });
        }
        if let Some(gradient) = &s.background_gradient {
            frame.commands.push(match gradient {
                refract_core::Gradient::Linear(gradient) => DrawCommand::LinearGradient {
                    rect,
                    radius: s.radius,
                    gradient: gradient.clone(),
                },
                refract_core::Gradient::Radial(gradient) => DrawCommand::RadialGradient {
                    rect,
                    radius: s.radius,
                    gradient: gradient.clone(),
                },
            });
        }
        if let Some(border) = s.border {
            frame.commands.push(DrawCommand::Border {
                rect,
                radius: s.radius,
                border,
            });
        }
        // Replaced image content lives inside padding/borders. Cover may extend
        // beyond that box, so clip it independently of ancestor clipping.
        if entry.element.kind == ElementKind::Image {
            let content = Rect {
                x: rect.x + layout.border.left + layout.padding.left,
                y: rect.y + layout.border.top + layout.padding.top,
                width: (rect.width
                    - layout.border.left
                    - layout.border.right
                    - layout.padding.left
                    - layout.padding.right)
                    .max(0.0),
                height: (rect.height
                    - layout.border.top
                    - layout.border.bottom
                    - layout.padding.top
                    - layout.padding.bottom)
                    .max(0.0),
            };
            frame.commands.push(DrawCommand::PushInsetLayer {
                rect,
                radius: s.radius,
                insets: Insets {
                    top: layout.border.top + layout.padding.top,
                    right: layout.border.right + layout.padding.right,
                    bottom: layout.border.bottom + layout.padding.bottom,
                    left: layout.border.left + layout.padding.left,
                },
            });
            if let Some(image) = entry.image.as_ref().and_then(|resource| {
                if let ImageState::Ready(image) = &resource.borrow().state {
                    Some(image.clone())
                } else {
                    None
                }
            }) {
                let destination = image_destination(
                    content,
                    image.width() as f32,
                    image.height() as f32,
                    s.image_fit,
                );
                frame.commands.push(DrawCommand::Image {
                    rect: destination,
                    image,
                });
            } else {
                frame.commands.push(DrawCommand::Fill {
                    rect: content,
                    radius: 0.0.into(),
                    color: Color::rgb(40, 42, 46),
                });
            }
        }
        if let Some(editor) = &mut entry.editor {
            let left = layout.border.left + layout.padding.left;
            let top = layout.border.top + layout.padding.top;
            editor.paint(
                Rect {
                    x: rect.x + left,
                    y: rect.y + top,
                    width: (rect.width - left - layout.border.right - layout.padding.right)
                        .max(0.0),
                    height: (rect.height - top - layout.border.bottom - layout.padding.bottom)
                        .max(0.0),
                },
                self.focus == Some(id),
                &mut self.text,
                frame,
            );
        }
        let mut text = entry.text.borrow_mut();
        if let Some(block) = text.block.as_mut() {
            let left = layout.border.left + layout.padding.left;
            let top = layout.border.top + layout.padding.top;
            let width = (rect.width - left - layout.border.right - layout.padding.right).max(0.0);
            block.measure(Some(width), entry.typography.align);
            block.prepare_ellipsis(&mut self.text);
            if block.truncate {
                frame.commands.push(DrawCommand::PushLayer {
                    clip: Rect {
                        x: rect.x + left,
                        y: rect.y + top,
                        width,
                        height: (rect.height - top - layout.border.bottom - layout.padding.bottom)
                            .max(0.0),
                    },
                    radius: 0.0.into(),
                    opacity: 1.0,
                });
            }
            for run in block.glyph_runs() {
                frame.commands.push(DrawCommand::Glyphs {
                    origin: Point {
                        x: rect.x + left,
                        y: rect.y + top,
                    },
                    color: if entry.element.kind == ElementKind::Image {
                        Color::rgb(220, 224, 220)
                    } else {
                        entry.typography.color
                    },
                    run: run.clone(),
                });
            }
            if block.truncate {
                frame.commands.push(DrawCommand::PopLayer);
            }
        }
        drop(text);
        if entry.element.kind == ElementKind::Image {
            frame.commands.push(DrawCommand::PopLayer);
        }
        let origin = Point {
            x: rect.x - entry.scroll.x,
            y: rect.y - entry.scroll.y,
        };
        let children = entry.children.clone();
        // A translucent black tint matches the web button's subtle darkening.
        // Paint it after descendants so image/text content gets feedback too.
        let feedback = (self.hover == Some(id) && !entry.element.disabled)
            .then_some((s.radius, if self.pressed == Some(id) { 26 } else { 13 }));
        let has_opacity = s.opacity.is_some() || has_effects;
        let ring = if self.focus == Some(id)
            && (self.keyboard_focus || entry.element.kind == ElementKind::TextInput)
        {
            Some(s.focus_ring.unwrap_or_default())
        } else if focus_within {
            s.focus_within
        } else {
            None
        };
        let radius = s.radius;
        for child in children {
            self.paint(child, origin, clip, frame)?;
        }
        if let Some((radius, alpha)) = feedback {
            frame.commands.push(DrawCommand::Fill {
                rect,
                radius,
                color: Color(0, 0, 0, alpha),
            });
        }
        self.paint_scrollbars(id, frame);
        if clipped {
            frame.commands.push(DrawCommand::PopLayer);
        }
        // Paint after descendants and outside our own clip, while retaining
        // ancestor clipping and group opacity. The stroke occupies its outer box.
        if let Some(ring) = ring.filter(|ring| ring.width > 0.0) {
            let ring = if self.keyboard_focus { ring } else { ring.pointer() };
            let outset = ring.offset + ring.width;
            frame.commands.push(DrawCommand::Stroke {
                rect: Rect { x: rect.x - outset, y: rect.y - outset,
                    width: rect.width + outset * 2.0, height: rect.height + outset * 2.0 },
                radius: refract_core::CornerRadii {
                    top_left: radius.top_left + outset,
                    top_right: radius.top_right + outset,
                    bottom_right: radius.bottom_right + outset,
                    bottom_left: radius.bottom_left + outset,
                },
                width: ring.width,
                color: ring.color,
            });
        }
        if has_opacity {
            frame.commands.push(DrawCommand::PopLayer);
        }
        Ok(())
    }
}

/// CSS-like replaced-element measurement: one definite axis preserves the intrinsic ratio.
fn image_measure(
    width: f32,
    height: f32,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> Size<f32> {
    let ratio = width / height.max(1.0);
    match (known.width, known.height) {
        (Some(width), Some(height)) => Size { width, height },
        (Some(width), None) => Size {
            width,
            height: width / ratio,
        },
        (None, Some(height)) => Size {
            width: height * ratio,
            height,
        },
        (None, None) => {
            let width = match available.width {
                AvailableSpace::Definite(max) => width.min(max.max(0.0)),
                AvailableSpace::MinContent => 0.0,
                AvailableSpace::MaxContent => width,
            };
            Size {
                width,
                height: width / ratio,
            }
        }
    }
}

pub(super) fn image_destination(bounds: Rect, width: f32, height: f32, fit: ImageFit) -> Rect {
    if fit == ImageFit::Stretch {
        return bounds;
    }
    let x_scale = bounds.width / width;
    let y_scale = bounds.height / height;
    let scale = if fit == ImageFit::Cover {
        x_scale.max(y_scale)
    } else {
        x_scale.min(y_scale)
    };
    let width = width * scale;
    let height = height * scale;
    Rect {
        x: bounds.x + (bounds.width - width) / 2.0,
        y: bounds.y + (bounds.height - height) / 2.0,
        width,
        height,
    }
}
