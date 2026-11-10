//! Insert an explicit base URL without rewriting application scripts or asset URLs.
use crate::Result;

/// Replace one explicit app marker, ignoring comments spelled inside raw text.
pub fn with_prerender(html: &str, fragment: &str) -> Result<String> {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0;
    let mut raw = None;
    let mut marker = None;
    while let Some(offset) = lower[cursor..].find('<') {
        let start = cursor + offset;
        if raw.is_none() && lower[start..].starts_with("<!--") {
            let end = start
                + 4
                + lower[start + 4..]
                    .find("-->")
                    .ok_or("unclosed HTML comment")?
                + 3;
            if &html[start..end] == "<!--refract:app-->" {
                if marker.replace(start..end).is_some() {
                    return Err(
                        "--prerender requires exactly one <!--refract:app--> inside the app host"
                            .into(),
                    );
                }
            }
            cursor = end;
            continue;
        }
        let closing = bytes.get(start + 1) == Some(&b'/');
        let name_start = start + if closing { 2 } else { 1 };
        let mut name_end = name_start;
        while bytes.get(name_end).is_some_and(u8::is_ascii_alphanumeric) {
            name_end += 1;
        }
        let name = &lower[name_start..name_end];
        if let Some(raw_name) = raw {
            if !closing || name != raw_name {
                cursor = name_end;
                continue;
            }
            raw = None;
        }
        let mut end = name_end;
        let mut quote = None;
        while let Some(&c) = bytes.get(end) {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                if c == b'>' {
                    break;
                }
                if c == b'\'' || c == b'"' {
                    quote = Some(c);
                }
            }
            end += 1;
        }
        if end == bytes.len() {
            break;
        }
        cursor = end + 1;
        if !closing && matches!(name, "script" | "style" | "textarea" | "title") {
            raw = Some(name);
        }
    }
    let marker = marker.ok_or(
        "--prerender requires <!--refract:app--> inside the app host in public/index.html",
    )?;
    Ok(format!(
        "{}{}{}",
        &html[..marker.start],
        fragment,
        &html[marker.end..]
    ))
}

pub fn with_base_path(html: &str, base: &str) -> Result<String> {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0;
    let mut head = None;
    let mut raw: Option<&str> = None;
    while let Some(relative) = lower[cursor..].find('<') {
        let start = cursor + relative;
        if raw.is_none() && lower[start..].starts_with("<!--") {
            cursor = start
                + 4
                + lower[start + 4..]
                    .find("-->")
                    .ok_or("unclosed HTML comment in public/index.html")?
                + 3;
            continue;
        }
        let closing = bytes.get(start + 1) == Some(&b'/');
        let name_start = start + if closing { 2 } else { 1 };
        let mut name_end = name_start;
        while bytes.get(name_end).is_some_and(u8::is_ascii_alphanumeric) {
            name_end += 1;
        }
        let name = &lower[name_start..name_end];
        if let Some(raw_name) = raw {
            if !closing || name != raw_name {
                cursor = name_end;
                continue;
            }
            raw = None;
        }
        let mut end = name_end;
        let mut quote = None;
        while let Some(&c) = bytes.get(end) {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                if c == b'>' {
                    break;
                }
                if c == b'\'' || c == b'"' {
                    quote = Some(c);
                }
            }
            end += 1;
        }
        if end == bytes.len() {
            break;
        }
        cursor = end + 1;
        match (closing, name) {
            (false, "head") => head = Some(cursor),
            (false, "base") => return Err(
                "--base-path requires removing the existing <base> element from public/index.html"
                    .into(),
            ),
            (true, "head") => {
                if let Some(position) = head {
                    return Ok(format!(
                        "{}\n<base href=\"{base}\">{}",
                        &html[..position],
                        &html[position..]
                    ));
                }
                break;
            }
            (false, "script" | "style" | "title" | "textarea") => raw = Some(name),
            _ => {}
        }
    }
    Err("--base-path requires an explicit <head>...</head> in public/index.html".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_fragment_only_at_an_actual_unique_comment() {
        let html = "<script>const s='<!--refract:app-->';</script><main><!--refract:app--></main>";
        assert_eq!(
            with_prerender(html, "<div>hi</div>").unwrap(),
            "<script>const s='<!--refract:app-->';</script><main><div>hi</div></main>"
        );
        assert!(with_prerender("<main></main>", "x").is_err());
        assert!(with_prerender("<!--refract:app--><!--refract:app-->", "x").is_err());
    }

    #[test]
    fn base_precedes_relative_urls_without_changing_scripts_or_comments() {
        let html = "<!doctype html><!-- <head> --><HTML><HEAD data-note='>'><title>A <base> title</title><script>const s = '<base href=\"fake\">';</script><link href='style.css'></HEAD><body>é</body></HTML>";
        let output = with_base_path(html, "/demo/").unwrap();
        assert_eq!(
            output,
            html.replacen(
                "<HEAD data-note='>'>",
                "<HEAD data-note='>'>\n<base href=\"/demo/\">",
                1
            )
        );
        assert!(with_base_path("<head><BASE href='/old/'></head>", "/new/").is_err());
        assert!(with_base_path("<body>no explicit head</body>", "/").is_err());
    }
}
