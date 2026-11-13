//! SVG lowering shared by browser rendering and deterministic prerendering.

use refract_core::{DrawingCommand, FillRule, LineCap, LineJoin, Paint, Path, PathElement, Size};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct SvgNode {
    tag: &'static str,
    attrs: BTreeMap<&'static str, String>,
    children: Vec<SvgNode>,
}

impl SvgNode {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            attrs: BTreeMap::new(),
            children: vec![],
        }
    }

    fn attr(mut self, key: &'static str, value: impl ToString) -> Self {
        self.attrs.insert(key, value.to_string());
        self
    }

    #[cfg(target_arch = "wasm32")]
    pub fn position(&mut self, x: f32, y: f32) {
        self.attrs
            .entry("style")
            .or_default()
            .push_str(&format!(";left:{x}px;top:{y}px"));
    }

    pub fn html(&self) -> String {
        let mut result = format!("<{}", self.tag);
        for (key, value) in &self.attrs {
            result.push_str(&format!(" {key}=\"{}\"", crate::html::escape(value)));
        }
        result.push('>');
        for child in &self.children {
            result.push_str(&child.html());
        }
        result.push_str(&format!("</{}>", self.tag));
        result
    }
}

fn path_data(path: &Path) -> String {
    let mut data = String::new();
    for el in path.elements() {
        data.push_str(&match *el {
            PathElement::MoveTo(p) => format!("M{} {}", p.x, p.y),
            PathElement::LineTo(p) => format!("L{} {}", p.x, p.y),
            PathElement::QuadTo(a, b) => format!("Q{} {} {} {}", a.x, a.y, b.x, b.y),
            PathElement::CubicTo(a, b, c) => {
                format!("C{} {} {} {} {} {}", a.x, a.y, b.x, b.y, c.x, c.y)
            }
            PathElement::Close => "Z".into(),
        });
    }
    data
}

fn rule(rule: FillRule) -> &'static str {
    match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

fn color(c: refract_core::Color) -> String {
    format!("rgba({},{},{},{})", c.0, c.1, c.2, f64::from(c.3) / 255.0)
}

struct Builder {
    prefix: String,
    next: usize,
    defs: Vec<SvgNode>,
}

impl Builder {
    fn id(&mut self) -> String {
        let id = format!("{}-{}", self.prefix, self.next);
        self.next += 1;
        id
    }

    fn paint(&mut self, paint: &Paint) -> String {
        let (mut gradient, stops) = match paint {
            Paint::Solid(c) => return color(*c),
            Paint::Linear { start, end, stops } => (
                SvgNode::new("linearGradient")
                    .attr("x1", start.x)
                    .attr("y1", start.y)
                    .attr("x2", end.x)
                    .attr("y2", end.y),
                stops,
            ),
            Paint::Radial {
                center,
                radius,
                stops,
            } => (
                SvgNode::new("radialGradient")
                    .attr("cx", center.x)
                    .attr("cy", center.y)
                    .attr("r", radius),
                stops,
            ),
        };
        let id = self.id();
        gradient = gradient
            .attr("id", &id)
            .attr("gradientUnits", "userSpaceOnUse")
            .attr("color-interpolation", "sRGB");
        gradient.children = stops
            .iter()
            .map(|s| {
                SvgNode::new("stop")
                    .attr("offset", s.offset)
                    .attr(
                        "stop-color",
                        format!("rgb({},{},{})", s.color.0, s.color.1, s.color.2),
                    )
                    .attr("stop-opacity", f64::from(s.color.3) / 255.0)
            })
            .collect();
        self.defs.push(gradient);
        format!("url(#{id})")
    }

    fn commands(&mut self, commands: &[DrawingCommand]) -> Vec<SvgNode> {
        commands
            .iter()
            .map(|command| match command {
                DrawingCommand::Fill {
                    path,
                    paint,
                    rule: r,
                } => SvgNode::new("path")
                    .attr("d", path_data(path))
                    .attr("fill", self.paint(paint))
                    .attr("fill-rule", rule(*r)),
                DrawingCommand::Stroke { path, paint, style } => SvgNode::new("path")
                    .attr("d", path_data(path))
                    .attr("fill", "none")
                    .attr("stroke", self.paint(paint))
                    .attr("stroke-width", style.width)
                    .attr(
                        "stroke-linecap",
                        match style.line_cap {
                            LineCap::Butt => "butt",
                            LineCap::Round => "round",
                            LineCap::Square => "square",
                        },
                    )
                    .attr(
                        "stroke-linejoin",
                        match style.line_join {
                            LineJoin::Miter => "miter",
                            LineJoin::Round => "round",
                            LineJoin::Bevel => "bevel",
                        },
                    )
                    .attr("stroke-miterlimit", style.miter_limit)
                    .attr(
                        "stroke-dasharray",
                        if style.dash.is_empty() {
                            "none".into()
                        } else {
                            style
                                .dash
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(" ")
                        },
                    )
                    .attr("stroke-dashoffset", style.dash_offset),
                DrawingCommand::Group {
                    transform,
                    clip,
                    layer,
                    commands,
                } => {
                    let mut outer = SvgNode::new("g").attr(
                        "transform",
                        format!(
                            "matrix({})",
                            transform
                                .0
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(" ")
                        ),
                    );
                    if let Some((path, r)) = clip {
                        let id = self.id();
                        let mut def = SvgNode::new("clipPath")
                            .attr("id", &id)
                            .attr("clipPathUnits", "userSpaceOnUse");
                        def.children.push(
                            SvgNode::new("path")
                                .attr("d", path_data(path))
                                .attr("clip-rule", rule(*r)),
                        );
                        self.defs.push(def);
                        outer = outer.attr("clip-path", format!("url(#{id})"));
                    }
                    let mut content = SvgNode::new("g");
                    content.children = self.commands(commands);
                    if let Some(layer) = layer {
                        let bounds = refract_core::drawing_bounds(commands)
                            .unwrap_or_default()
                            .inflate(layer.blur * 3.0 + 1.0);
                        if layer.blur > 0.0 {
                            let id = self.id();
                            let mut filter = SvgNode::new("filter")
                                .attr("id", &id)
                                .attr("filterUnits", "userSpaceOnUse")
                                .attr("x", bounds.x)
                                .attr("y", bounds.y)
                                .attr("width", bounds.width)
                                .attr("height", bounds.height)
                                .attr("color-interpolation-filters", "sRGB");
                            filter.children.push(
                                SvgNode::new("feGaussianBlur").attr("stdDeviation", layer.blur),
                            );
                            self.defs.push(filter);
                            content = content.attr("filter", format!("url(#{id})"));
                        }
                        // Separate wrappers make blur -> mask -> opacity ordering explicit.
                        let mut composite = SvgNode::new("g").attr("opacity", layer.opacity).attr(
                            "style",
                            format!(
                                "isolation:isolate;mix-blend-mode:{}",
                                match layer.blend_mode {
                                    refract_core::BlendMode::Normal => "normal",
                                    refract_core::BlendMode::Multiply => "multiply",
                                    refract_core::BlendMode::Screen => "screen",
                                    refract_core::BlendMode::Plus => "plus-lighter",
                                }
                            ),
                        );
                        if let Some(mask) = &layer.mask {
                            let id = self.id();
                            let mut def = SvgNode::new("mask")
                                .attr("id", &id)
                                .attr("maskUnits", "userSpaceOnUse")
                                .attr("maskContentUnits", "userSpaceOnUse")
                                .attr("style", "mask-type:alpha")
                                .attr("x", bounds.x)
                                .attr("y", bounds.y)
                                .attr("width", bounds.width)
                                .attr("height", bounds.height);
                            def.children.push(
                                SvgNode::new("path")
                                    .attr("d", path_data(&mask.path))
                                    .attr("fill", self.paint(&mask.paint)),
                            );
                            self.defs.push(def);
                            composite = composite.attr("mask", format!("url(#{id})"));
                        }
                        composite.children.push(content);
                        outer.children.push(composite);
                    } else {
                        outer.children.push(content);
                    }
                    outer
                }
            })
            .collect()
    }
}

pub(crate) fn scene(commands: &[DrawingCommand], size: Size, prefix: &str) -> SvgNode {
    let mut builder = Builder {
        prefix: prefix.into(),
        next: 0,
        defs: vec![],
    };
    let children = builder.commands(commands);
    let mut defs = SvgNode::new("defs");
    defs.children = builder.defs;
    let mut root = SvgNode::new("svg")
        .attr("xmlns", "http://www.w3.org/2000/svg")
        .attr("width", size.width)
        .attr("height", size.height)
        .attr(
            "viewBox",
            format!("0 0 {} {}", size.width.max(0.001), size.height.max(0.001)),
        )
        .attr("aria-hidden", "true")
        .attr(
            "style",
            "position:absolute;display:block;overflow:hidden;pointer-events:none;isolation:isolate",
        );
    root.children.push(defs);
    root.children.extend(children);
    root
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn update(
    parent: &web_sys::Element,
    tree: &SvgNode,
) -> Result<u64, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;

    fn sync(
        parent: &web_sys::Element,
        index: u32,
        tree: &SvgNode,
        calls: &mut u64,
    ) -> Result<(), wasm_bindgen::JsValue> {
        // Stable command order lets animated attributes reuse existing DOM nodes.
        let existing = parent.children().item(index);
        let node = if let Some(node) = existing.as_ref().filter(|e| e.local_name() == tree.tag) {
            node.clone()
        } else {
            let node = parent
                .owner_document()
                .unwrap()
                .create_element_ns(Some("http://www.w3.org/2000/svg"), tree.tag)?;
            if let Some(old) = existing {
                parent.replace_child(&node, &old)?;
            } else {
                parent.append_child(&node)?;
            }
            *calls += 1;
            node
        };
        for key in node
            .get_attribute_names()
            .iter()
            .filter_map(|v| v.as_string())
        {
            if !tree.attrs.contains_key(key.as_str()) {
                node.remove_attribute(&key)?;
                *calls += 1;
            }
        }
        for (key, value) in &tree.attrs {
            if node.get_attribute(key).as_ref() != Some(value) {
                node.set_attribute(key, value)?;
                *calls += 1;
            }
        }
        for (i, child) in tree.children.iter().enumerate() {
            sync(&node, i as u32, child, calls)?;
        }
        while node.children().length() > tree.children.len() as u32 {
            node.last_element_child().unwrap().remove();
            *calls += 1;
        }
        Ok(())
    }
    let mut calls = 0;
    sync(parent.unchecked_ref(), 0, tree, &mut calls)?;
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::*;

    #[test]
    fn masks_filters_and_gradients_use_local_coordinates_and_unique_ids() {
        let mut g = GraphicsContext::new();
        let p = Path::rectangle(Rect::new(0.0, 0.0, 50.0, 30.0));
        let gradient = Paint::linear(
            Point::ZERO,
            Point::new(50.0, 0.0),
            vec![
                GradientStop::new(0.0, Color::WHITE),
                GradientStop::new(1.0, Color(0, 0, 0, 0)),
            ],
        );
        g.layer(
            Layer::new()
                .blur(3.0)
                .mask(Mask::new(p.clone(), gradient.clone())),
            |g| g.fill(&p, gradient),
        );
        let commands = g.finish().unwrap();
        let a = scene(&commands, Size::new(50.0, 30.0), "a").html();
        let b = scene(&commands, Size::new(50.0, 30.0), "b").html();
        assert!(a.contains("mask-type:alpha"));
        assert!(a.contains("gradientUnits=\"userSpaceOnUse\""));
        assert!(a.contains("stdDeviation=\"3\""));
        assert!(!b.contains("url(#a-"));
        assert!(a.contains("stop-opacity=\"0\""));
    }
}

#[cfg(test)]
mod prerender_tests {
    use refract_core::prelude::*;

    #[test]
    fn prerender_emits_vectors_and_reports_invalid_drawings() {
        let html = crate::prerender(
            canvas(|g, frame| g.fill(&Path::ellipse(frame.bounds), Color::BLUE))
                .width(100.0)
                .height(80.0),
            Default::default(),
        )
        .unwrap();
        assert!(html.contains("<svg"));
        assert!(html.contains("viewBox=\"0 0 100 80\""));
        assert!(html.contains("<path"));
        assert!(crate::prerender(
            canvas(|g, _| g.stroke(&Path::new(), Color::BLUE, StrokeStyle::new(-1.0))),
            Default::default()
        )
        .is_err());
    }
}
