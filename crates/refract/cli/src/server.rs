use crate::Result;
use serde::Serialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Clone, Serialize)]
pub struct BuildState {
    pub session: String,
    pub revision: u64,
    pub building: bool,
    pub error: Option<String>,
    #[serde(skip)]
    pub pkg: Option<PathBuf>,
}

pub type SharedState = Arc<Mutex<BuildState>>;

pub struct DevServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub url: String,
}

impl DevServer {
    pub fn start(public: PathBuf, port: u16, state: SharedState) -> Result<Self> {
        let server = Server::http(("127.0.0.1", port))?;
        let address = server
            .server_addr()
            .to_ip()
            .ok_or("expected TCP listener")?;
        let port = address.port();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => respond(request, &public, port, &state),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("development server: {error}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
            url: format!("http://127.0.0.1:{port}"),
        })
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn respond(request: Request, public: &Path, port: u16, state: &SharedState) {
    let host = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str());
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !host.is_some_and(|h| allowed.iter().any(|a| a == h)) {
        send(
            request,
            403,
            "text/plain",
            b"Use the localhost development URL".to_vec(),
        );
        return;
    }
    if request.method() != &Method::Get && request.method() != &Method::Head {
        send(request, 405, "text/plain", b"GET and HEAD only".to_vec());
        return;
    }
    let url = request.url().split('?').next().unwrap_or("/");
    if url == "/__refract/status" {
        let body = serde_json::to_vec(&*state.lock().unwrap()).unwrap();
        send(request, 200, "application/json", body);
        return;
    }
    if url == "/__refract/client.js" {
        send(
            request,
            200,
            "text/javascript",
            include_bytes!("../templates/client.js").to_vec(),
        );
        return;
    }
    // Hold the generation until its file has been read, so a successful rebuild
    // cannot remove that generation between path lookup and read.
    let snapshot = state.lock().unwrap();
    let mut path = if let Some(relative) = url.strip_prefix("/pkg/") {
        snapshot
            .pkg
            .as_deref()
            .and_then(|root| resolve_path(root, relative))
    } else {
        resolve_path(public, url.trim_start_matches('/'))
    };
    let fallback = path.is_none() && is_app_navigation(&request, url);
    if fallback {
        path = resolve_path(public, "index.html");
    }
    let Some(path) = path else {
        send(request, 404, "text/plain", b"Not found".to_vec());
        return;
    };
    let Ok(mut body) = fs::read(&path) else {
        send(request, 404, "text/plain", b"Not found".to_vec());
        return;
    };
    let content_type = mime(&path);
    if content_type == "text/html; charset=utf-8" {
        if let Ok(html) = String::from_utf8(body.clone()) {
            let html = if fallback {
                match crate::html::with_default_base_path(&html, "/") {
                    Ok(html) => html,
                    Err(error) => {
                        send(request, 500, "text/plain", error.to_string().into_bytes());
                        return;
                    }
                }
            } else {
                html
            };
            let script = format!("<script src=\"/__refract/client.js\" data-session=\"{}\" data-revision=\"{}\" defer></script>", snapshot.session, snapshot.revision);
            let html = if let Some(position) = html.to_ascii_lowercase().find("</head>") {
                format!("{}{script}{}", &html[..position], &html[position..])
            } else {
                format!("{script}{html}")
            };
            body = html.into_bytes();
        }
    }
    drop(snapshot);
    send(request, 200, content_type, body);
}

fn is_app_navigation(request: &Request, url: &str) -> bool {
    let Some(relative) = decode_path(url.trim_start_matches('/')) else {
        return false;
    };
    if relative.extension().is_some()
        || relative.starts_with("pkg")
        || relative.starts_with("__refract")
    {
        return false;
    }
    if let Some(destination) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Sec-Fetch-Dest"))
    {
        if !matches!(destination.value.as_str(), "document" | "iframe" | "frame") {
            return false;
        }
    }
    request
        .headers()
        .iter()
        .filter(|header| header.field.equiv("Accept"))
        .any(|header| {
            header.value.as_str().split(',').any(|range| {
                let mut parts = range.split(';');
                parts
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
                    && parts.all(|parameter| {
                        let Some((name, value)) = parameter.trim().split_once('=') else {
                            return true;
                        };
                        !name.trim().eq_ignore_ascii_case("q")
                            || value
                                .trim()
                                .parse::<f32>()
                                .is_ok_and(|q| q > 0.0 && q <= 1.0)
                    })
            })
        })
}

fn send(request: Request, status: u16, mime: &str, body: Vec<u8>) {
    let response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Content-Type", mime).unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap());
    let _ = request.respond(response);
}

fn resolve_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = decode_path(relative)?;
    let root = root.canonicalize().ok()?;
    let mut path = root.join(relative);
    if path.is_dir() {
        path.push("index.html");
    }
    let path = path.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file()).then_some(path)
}

fn decode_path(relative: &str) -> Option<PathBuf> {
    let bytes = relative.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let relative = String::from_utf8(decoded).ok()?;
    if relative.contains(['\\', '\0']) {
        return None;
    }
    let relative = Path::new(&relative);
    if relative.components().any(|part| match part {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => true,
    }) {
        return None;
    }
    Some(relative.into())
}

fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "wasm" => "application/wasm",
        "css" => "text/css",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn request(server: &DevServer, method: &str, path: &str, headers: &str) -> String {
        let address = server.url.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\n{headers}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn state() -> SharedState {
        Arc::new(Mutex::new(BuildState {
            session: "test".into(),
            revision: 3,
            building: false,
            error: None,
            pkg: None,
        }))
    }

    #[test]
    fn serves_app_routes_with_root_relative_assets_and_reload_client() {
        let temp = tempfile::tempdir().unwrap();
        let html = include_str!("../templates/index.html");
        fs::write(temp.path().join("index.html"), html).unwrap();
        fs::write(temp.path().join("style.css"), "body {}").unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/index.html"), "<head></head>Docs").unwrap();
        let pkg = tempfile::tempdir().unwrap();
        fs::write(pkg.path().join("app.js"), "export default {};").unwrap();
        let state = state();
        state.lock().unwrap().pkg = Some(pkg.path().into());
        let server = DevServer::start(temp.path().into(), 0, state).unwrap();
        let headers =
            "Accept: text/html,application/xhtml+xml;q=0.9\r\nSec-Fetch-Dest: document\r\n";
        for path in [
            "/conversations/42",
            "/conversations/42/?tab=messages",
            "/caf%C3%A9/42",
        ] {
            let response = request(&server, "GET", path, headers);
            assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
            assert!(response.contains("text/html; charset=utf-8"));
            assert!(response.contains("<base href=\"/\">"));
            assert!(response.contains("import init, { start } from './pkg/"));
            assert!(response.contains("data-revision=\"3\""));
        }
        let head = request(&server, "HEAD", "/conversations/42", headers);
        assert!(head.starts_with("HTTP/1.1 200"));
        assert!(head.ends_with("\r\n\r\n"));
        assert!(request(&server, "GET", "/pkg/app.js", "").ends_with("export default {};"));
        assert!(request(&server, "GET", "/style.css", headers).ends_with("body {}"));
        let docs = request(&server, "GET", "/docs/", headers);
        assert!(docs.ends_with("Docs"));
        assert!(!docs.contains("<base"));
        assert!(!request(&server, "GET", "/", headers).contains("<base"));
        assert_eq!(
            fs::read_to_string(temp.path().join("index.html")).unwrap(),
            html
        );
    }

    #[test]
    fn fallback_does_not_mask_assets_endpoints_or_invalid_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("index.html"), "<head></head>App").unwrap();
        let server = DevServer::start(temp.path().into(), 0, state()).unwrap();
        for path in [
            "/missing.js",
            "/missing.css",
            "/missing.wasm",
            "/missing.png",
            "/missing%2Ejs",
            "/pkg/missing",
            "/pkg",
            "/%70kg/missing",
            "/__refract/missing",
            "/__refract",
            "/%5f_refract/missing",
            "/../secret",
            "/%2e%2e/secret",
            "/.env",
            "/%zz",
            "/%2fsecret",
            "/bad%00path",
            "/bad%5cpath",
        ] {
            let response = request(&server, "GET", path, "Accept: text/html\r\n");
            assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
        }
        for headers in [
            "",
            "Accept: */*\r\n",
            "Accept: application/json\r\n",
            "Accept: text/html;q=0,*/*;q=1\r\n",
            "Accept: text/html\r\nSec-Fetch-Dest: empty\r\n",
            "Accept: text/html\r\nSec-Fetch-Dest: script\r\n",
        ] {
            assert!(request(&server, "GET", "/missing", headers).starts_with("HTTP/1.1 404"));
        }
        assert!(
            request(&server, "POST", "/missing", "Accept: text/html\r\n")
                .starts_with("HTTP/1.1 405")
        );
        fs::remove_file(temp.path().join("index.html")).unwrap();
        assert!(request(&server, "GET", "/missing", "Accept: text/html\r\n")
            .starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn refuses_traversal_and_symlinks_outside_public() {
        let temp = tempfile::tempdir().unwrap();
        let public = temp.path().join("public");
        fs::create_dir(&public).unwrap();
        fs::write(public.join("index.html"), "hello").unwrap();
        fs::write(temp.path().join("secret"), "secret").unwrap();
        assert!(resolve_path(&public, "").is_some());
        for path in [
            "../secret",
            "%2e%2e/secret",
            "%2fsecret",
            "%zz",
            ".env",
            "..\\secret",
        ] {
            assert!(resolve_path(&public, path).is_none(), "{path}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path().join("secret"), public.join("link")).unwrap();
            assert!(resolve_path(&public, "link").is_none());
        }
    }

    #[test]
    fn serves_reload_client_status_and_wasm_without_writing_public() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("index.html"),
            "<head></head><body>Hello</body>",
        )
        .unwrap();
        fs::write(temp.path().join("app.wasm"), b"\0asm").unwrap();
        let state = Arc::new(Mutex::new(BuildState {
            session: "test".into(),
            revision: 3,
            building: false,
            error: Some("bad build".into()),
            pkg: Some(temp.path().into()),
        }));
        let server = DevServer::start(temp.path().into(), 0, state).unwrap();
        let address = server.url.strip_prefix("http://").unwrap();
        let get = |path: &str| {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        assert!(get("/").contains("data-revision=\"3\""));
        assert!(get("/__refract/status").contains("bad build"));
        let wasm = get("/pkg/app.wasm");
        assert!(wasm.contains("application/wasm"));
        assert!(wasm.contains("no-store"));
        assert!(!fs::read_to_string(temp.path().join("index.html"))
            .unwrap()
            .contains("script"));
    }
}
