//! Contenteditable adapter. The application owns text and formatting; the DOM
//! owns caret movement and composition. Never accept HTML as document content.
use refract_core::{TextSelection, TextSpan};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, Node};

fn flatten(
    node: &Node,
    root: bool,
    target: Option<(&Node, u32)>,
    text: &mut String,
    found: &mut Option<u32>,
) {
    // Browsers keep a solitary <br> (sometimes inside an empty formatting span)
    // after deleting the last character. It maintains the caret's line box; it
    // is not a newline in the document. Apply the same interpretation to value
    // and selection offsets so the next insertion still starts at offset zero.
    if root && is_empty_placeholder(node) {
        if let Some((target, _)) = target {
            if node.contains(Some(target)) {
                *found = Some(0);
            }
        }
        return;
    }
    if node.node_type() == Node::TEXT_NODE {
        let value = node.node_value().unwrap_or_default();
        if let Some((target, offset)) = target {
            if node == target {
                *found = Some(
                    text.encode_utf16().count() as u32
                        + offset.min(value.encode_utf16().count() as u32),
                );
            }
        }
        text.push_str(&value);
        return;
    }
    let element = node.dyn_ref::<web_sys::Element>();
    if element.is_some_and(|e| e.has_attribute("data-refract-tail")) {
        return;
    }
    if element.is_some_and(|e| e.tag_name() == "BR") {
        text.push('\n');
        return;
    }
    if !root
        && element.is_some_and(|e| matches!(e.tag_name().as_str(), "DIV" | "P"))
        && !text.is_empty()
        && !text.ends_with('\n')
    {
        text.push('\n');
    }
    let children = node.child_nodes();
    for index in 0..=children.length() {
        if let Some((target, offset)) = target {
            if node == target && offset == index {
                *found = Some(text.encode_utf16().count() as u32);
            }
        }
        if let Some(child) = children.item(index) {
            flatten(&child, false, target, text, found);
        }
    }
}

fn is_empty_placeholder(node: &Node) -> bool {
    if !node.text_content().unwrap_or_default().is_empty() {
        return false;
    }
    node.dyn_ref::<web_sys::Element>()
        .and_then(|element| element.query_selector_all("br").ok())
        .is_some_and(|breaks| breaks.length() == 1)
}

pub(super) fn value(node: &HtmlElement) -> String {
    let mut text = String::new();
    flatten(node, true, None, &mut text, &mut None);
    text
}

fn browser_selection(node: &HtmlElement) -> Option<web_sys::Selection> {
    let root = node.get_root_node();
    js_sys::Reflect::get(root.as_ref(), &"getSelection".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .and_then(|f| f.call0(root.as_ref()).ok())
        .and_then(|value| value.dyn_into::<web_sys::Selection>().ok())
        .or_else(|| node.owner_document()?.get_selection().ok().flatten())
}

pub(super) fn selection(node: &HtmlElement) -> Option<TextSelection> {
    let selection = browser_selection(node)?;
    let anchor = selection.anchor_node()?;
    let focus = selection.focus_node()?;
    let offset = |target: &Node, index| {
        let mut result = None;
        flatten(
            node,
            true,
            Some((target, index)),
            &mut String::new(),
            &mut result,
        );
        result
    };
    if let (Some(anchor), Some(focus)) = (
        offset(&anchor, selection.anchor_offset()),
        offset(&focus, selection.focus_offset()),
    ) {
        return Some(TextSelection { anchor, focus });
    }
    // Safari can retarget Selection endpoints to the shadow host. Composed
    // ranges explicitly opt into this root and expose the actual text nodes.
    let function = js_sys::Reflect::get(selection.as_ref(), &"getComposedRanges".into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let options = js_sys::Object::new();
    let roots = js_sys::Array::new();
    roots.push(&node.get_root_node());
    js_sys::Reflect::set(&options, &"shadowRoots".into(), &roots).ok()?;
    let ranges = js_sys::Array::from(&function.call1(selection.as_ref(), &options).ok()?);
    let range = ranges.get(0);
    let start = js_sys::Reflect::get(&range, &"startContainer".into())
        .ok()?
        .dyn_into::<Node>()
        .ok()?;
    let end = js_sys::Reflect::get(&range, &"endContainer".into())
        .ok()?
        .dyn_into::<Node>()
        .ok()?;
    let a = js_sys::Reflect::get(&range, &"startOffset".into())
        .ok()?
        .as_f64()? as u32;
    let f = js_sys::Reflect::get(&range, &"endOffset".into())
        .ok()?
        .as_f64()? as u32;
    let (a, f) = (offset(&start, a)?, offset(&end, f)?);
    let backward = js_sys::Reflect::get(selection.as_ref(), &"direction".into())
        .ok()
        .and_then(|v| v.as_string())
        .as_deref()
        == Some("backward");
    Some(if backward {
        TextSelection {
            anchor: f,
            focus: a,
        }
    } else {
        TextSelection {
            anchor: a,
            focus: f,
        }
    })
}

fn point(node: &Node, remaining: &mut u32) -> Option<(Node, u32)> {
    if node.node_type() == Node::TEXT_NODE {
        let length = node.node_value().unwrap_or_default().encode_utf16().count() as u32;
        if *remaining <= length {
            return Some((node.clone(), *remaining));
        }
        *remaining -= length;
    } else {
        for index in 0..node.child_nodes().length() {
            if let Some(result) = point(&node.child_nodes().item(index)?, remaining) {
                return Some(result);
            }
        }
    }
    None
}

pub(super) fn set_selection(node: &HtmlElement, range: TextSelection) {
    let active = node
        .get_root_node()
        .dyn_into::<web_sys::ShadowRoot>()
        .ok()
        .and_then(|root| root.active_element())
        .or_else(|| node.owner_document()?.active_element());
    if !active.is_some_and(|active| active.is_same_node(Some(node))) {
        return;
    }
    let Some(selection) = browser_selection(node) else {
        return;
    };
    let length = value(node).encode_utf16().count() as u32;
    let endpoint =
        |offset: u32| point(node, &mut offset.min(length)).unwrap_or((node.clone().into(), 0));
    let (anchor, a) = endpoint(range.anchor);
    let (focus, f) = endpoint(range.focus);
    let _ = selection.set_base_and_extent(&anchor, a, &focus, f);
}

fn byte(value: &str, target: u32) -> usize {
    let mut offset = 0;
    for (index, ch) in value.char_indices() {
        if offset >= target {
            return index;
        }
        offset += ch.len_utf16() as u32;
    }
    value.len()
}

pub(super) fn render(
    node: &HtmlElement,
    text: &str,
    spans: &[TextSpan],
    highlight: Option<refract_core::TextHighlight>,
) -> Result<(), wasm_bindgen::JsValue> {
    let selection = selection(node);
    let document = node.owner_document().expect("mounted rich editor");
    let length = text.encode_utf16().count() as u32;
    let mut boundaries = vec![0, length];
    for span in spans {
        boundaries.extend([span.start.min(length), span.end.min(length)]);
    }
    if let Some(refract_core::TextHighlight { range, .. }) = highlight {
        boundaries.extend([range.anchor.min(length), range.focus.min(length)]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let fragment = document.create_document_fragment();
    for range in boundaries.windows(2) {
        let value = &text[byte(text, range[0])..byte(text, range[1])];
        let bold = spans
            .iter()
            .any(|s| s.start <= range[0] && s.end >= range[1] && s.bold);
        let italic = spans
            .iter()
            .any(|s| s.start <= range[0] && s.end >= range[1] && s.italic);
        let underline = spans
            .iter()
            .any(|s| s.start <= range[0] && s.end >= range[1] && s.underline);
        let part = document.create_element("span")?;
        let mut style = String::new();
        if let Some(highlight) = highlight.filter(|highlight| {
            let s = highlight.range;
            s.anchor.min(s.focus) <= range[0] && s.anchor.max(s.focus) >= range[1]
        }) {
            style.push_str(&format!("background:{};", highlight.color.css()));
        }
        if bold {
            style.push_str("font-weight:700;");
        }
        if italic {
            style.push_str("font-style:italic;");
        }
        if underline {
            style.push_str("text-decoration:underline;");
        }
        part.set_attribute("style", &style)?;
        part.set_text_content(Some(value));
        fragment.append_child(&part)?;
    }
    if text.ends_with('\n') {
        let tail = document.create_element("br")?;
        tail.set_attribute("data-refract-tail", "")?;
        fragment.append_child(&tail)?;
    }
    node.set_text_content(None);
    node.append_child(&fragment)?;
    if let Some(selection) = selection {
        set_selection(node, selection);
    }
    Ok(())
}

/// Hit testing is read-only; it must not alter the browser's active selection.
pub(super) fn hit_offset(node: &HtmlElement, x: f64, y: f64) -> Option<u32> {
    let document = node.owner_document()?;
    let rect = node.get_bounding_client_rect();
    // Tagged geometry includes distant fields. Avoid walking their glyphs.
    if y < rect.top() {
        return Some(0);
    }
    if y > rect.bottom() {
        return Some(value(node).encode_utf16().count() as u32);
    }
    let x = x.clamp(
        rect.left() + 1.0,
        (rect.right() - 1.0).max(rect.left() + 1.0),
    );
    let y = y.clamp(
        rect.top() + 1.0,
        (rect.bottom() - 1.0).max(rect.top() + 1.0),
    );
    // Shadow roots must be explicitly included or browsers retarget the hit to
    // the host element, which has no offset inside this field.
    let options = js_sys::Object::new();
    let roots = js_sys::Array::new();
    roots.push(&node.get_root_node());
    let _ = js_sys::Reflect::set(&options, &"shadowRoots".into(), &roots);
    let modern = js_sys::Reflect::get(document.as_ref(), &"caretPositionFromPoint".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .and_then(|f| {
            f.call3(document.as_ref(), &x.into(), &y.into(), &options)
                .ok()
        })
        .and_then(|caret| caret.dyn_into::<web_sys::CaretPosition>().ok())
        .and_then(|caret| Some((caret.offset_node()?, caret.offset())));
    let legacy = || {
        js_sys::Reflect::get(document.as_ref(), &"caretRangeFromPoint".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .and_then(|f| f.call2(document.as_ref(), &x.into(), &y.into()).ok())
            .and_then(|range| range.dyn_into::<web_sys::Range>().ok())
            .and_then(|range| Some((range.start_container().ok()?, range.start_offset().ok()?)))
    };
    for (target, index) in modern.into_iter().chain(legacy()) {
        let mut offset = None;
        flatten(
            node,
            true,
            Some((&target, index)),
            &mut String::new(),
            &mut offset,
        );
        if offset.is_some() {
            return offset;
        }
    }
    // Older WebKit does not offer a shadow-aware caret API. DOM ranges still
    // expose shaped glyph geometry without disturbing the user's selection.
    let text = value(node);
    let range = document.create_range().ok()?;
    let mut best = (f64::INFINITY, 0);
    let mut offset = 0;
    for ch in text.chars() {
        let end = offset + ch.len_utf16() as u32;
        let mut start_remaining = offset;
        let mut end_remaining = end;
        if let (Some((start_node, a)), Some((end_node, b))) = (
            point(node, &mut start_remaining),
            point(node, &mut end_remaining),
        ) {
            range.set_start(&start_node, a).ok()?;
            range.set_end(&end_node, b).ok()?;
            let glyph = range.get_bounding_client_rect();
            let dy = (glyph.top() - y).max(0.0).max(y - glyph.bottom());
            for (edge, at) in [(glyph.left(), offset), (glyph.right(), end)] {
                let score = dy * dy + (edge - x).powi(2);
                if score < best.0 {
                    best = (score, at);
                }
            }
        }
        offset = end;
    }
    Some(best.1)
}
