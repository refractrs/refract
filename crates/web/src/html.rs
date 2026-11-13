//! Shared HTML vocabulary for prerendering and DOM adoption.
use base64::Engine;
use refract_core::{Element, ElementId, ElementKind, ImageSource, Interaction, Layout};
use std::collections::BTreeMap;

pub(crate) const BASE_STYLE: &str = include_str!("base.css");

pub(crate) fn tag(element: &Element) -> &'static str {
    match element.kind {
        ElementKind::Box | ElementKind::Canvas => "div",
        ElementKind::Text => "span",
        ElementKind::TextInput if element.multiline => "textarea",
        ElementKind::TextInput => "input",
        ElementKind::Image => "img",
    }
}

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn attributes(id: ElementId, e: &Element) -> BTreeMap<&'static str, String> {
    let mut attrs = BTreeMap::from([
        ("class", "refract-node".into()),
        ("data-refract-id", id.0.to_string()),
        (
            "data-refract-layout",
            match e.style.layout {
                Layout::Row => "row",
                Layout::Column => "column",
                Layout::Overlay => "overlay",
                _ => "box",
            }
            .into(),
        ),
        (
            "style",
            crate::style::css(e)
                .iter()
                .map(|(key, value)| format!("{key}:{value};"))
                .collect(),
        ),
    ]);
    for (key, value) in [
        ("aria-label", &e.label),
        ("role", &e.role),
        ("placeholder", &e.placeholder),
    ] {
        if let Some(value) = value {
            attrs.insert(key, value.clone());
        }
    }
    if let Some(level) = e.level {
        attrs.insert("aria-level", level.to_string());
    }
    if let Some(checked) = e.checked {
        attrs.insert("aria-checked", checked.to_string());
    }
    if e.style.fill_width {
        attrs.insert("data-fill-width", String::new());
    }
    if e.style.fill_height {
        attrs.insert("data-fill-height", String::new());
    }
    if e.focus_target
        || e.focusable.is_some()
        || e.interaction.is_some()
        || e.kind == ElementKind::TextInput
    {
        attrs.insert("tabindex", if e.is_focusable() { "0" } else { "-1" }.into());
    }
    if e.interaction == Some(Interaction::Activate) {
        attrs.insert("data-refract-activate", String::new());
        attrs.insert("aria-disabled", e.disabled.to_string());
    }
    if e.kind == ElementKind::TextInput {
        if e.disabled {
            attrs.insert("disabled", String::new());
        }
        if !e.multiline {
            attrs.insert("value", e.value.clone());
        }
    }
    if e.kind == ElementKind::Image {
        attrs.insert("alt", e.label.clone().unwrap_or_default());
    }
    if let Some(source) = &e.source {
        attrs.insert(
            "src",
            match source {
                ImageSource::Url(url) => url.to_string(),
                ImageSource::Embedded { format, bytes } => format!(
                    "data:{};base64,{}",
                    format.mime_type(),
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ),
            },
        );
    }
    attrs
}
