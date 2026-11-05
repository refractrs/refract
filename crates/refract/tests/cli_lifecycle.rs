//! Exercise real CLI processes with a deterministic toolchain, including failures
//! and interrupts, without requiring a window manager or compiling a GPU stack.
#![cfg(unix)]

use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Session {
    child: Child,
    directory: tempfile::TempDir,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        let _ = self.child.wait();
    }
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn session(platform: &str) -> Session {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for path in ["tools", "src", "public", "build output", "stdlib"] {
        fs::create_dir(root.join(path)).unwrap();
    }
    fs::write(root.join("stdlib/libstd-test.rlib"), "").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = 'fixture'\nversion = '0.1.0'\n",
    )
    .unwrap();
    fs::write(
        root.join("public/index.html"),
        "<head></head><body>fixture</body>",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "valid").unwrap();
    fs::write(root.join("build output/fixture.wasm"), b"\0asm").unwrap();
    executable(
        &root.join("build output/fixture"),
        "#!/bin/sh\necho $$ >> \"$FIXTURE/build output/app-pids\"\nwhile :; do sleep 1; done\n",
    );
    let target = json!({"name":"fixture", "kind": [if platform == "web" {"cdylib"} else {"bin"}], "crate_types": [if platform == "web" {"cdylib"} else {"bin"}]});
    fs::write(root.join("metadata.json"), json!({
        "packages": [
            {"id":"fixture-id", "name":"fixture", "version":"0.1.0", "manifest_path":root.join("Cargo.toml"), "source":null, "targets":[target], "features":{}, "default_run":null},
            {"id":"bindgen-id", "name":"wasm-bindgen", "version":"0.2.130", "manifest_path":root.join("registry/Cargo.toml"), "source":"registry", "targets":[], "features":{}, "default_run":null}
        ],
        "workspace_members":["fixture-id"], "workspace_root":root, "target_directory":root.join("build output")
    }).to_string()).unwrap();
    let artifact = json!({"reason":"compiler-artifact", "package_id":"fixture-id", "target":target,
        "executable":root.join("build output/fixture"), "filenames":[root.join("build output/fixture.wasm")]});
    fs::write(
        root.join("artifacts.json"),
        format!(
            "{}\n{artifact}\n",
            json!({"reason":"compiler-artifact", "package_id":"bindgen-id"})
        ),
    )
    .unwrap();
    executable(
        &root.join("tools/cargo"),
        r#"#!/bin/sh
if [ "$1" = metadata ]; then cat "$FIXTURE/metadata.json"; exit; fi
if [ "$1" = build ]; then
  if [ "$(cat "$FIXTURE/src/lib.rs")" = broken ]; then echo 'fixture compile failure' >&2; exit 1; fi
  cat "$FIXTURE/artifacts.json"
  exit
fi
exit 1
"#,
    );
    executable(&root.join("tools/rustc"), "#!/bin/sh\nif [ \"$1\" = -vV ]; then echo 'host: fixture-native'; else echo \"$FIXTURE/stdlib\"; fi\n");
    executable(
        &root.join("tools/wasm-bindgen"),
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'wasm-bindgen 0.2.130'; exit; fi
cp "$FIXTURE/build output/fixture.wasm" "$4/fixture_bg.wasm"
echo 'export default () => {};' > "$4/fixture.js"
"#,
    );
    let log = fs::File::create(root.join("build output/dev.log")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(root.join("tools"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_refract"))
        .args(["dev", "--platform", platform, "--port", "0", "--offline"])
        .current_dir(root)
        .env("FIXTURE", root)
        .env("PATH", path)
        .env("REFRACT_WASM_BINDGEN", root.join("tools/wasm-bindgen"))
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    Session { child, directory }
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn log(session: &Session) -> String {
    fs::read_to_string(session.directory.path().join("build output/dev.log")).unwrap()
}

fn status(address: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET /__refract/status HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[test]
fn web_keeps_successful_revision_on_failure_and_recovers() {
    let session = session("web");
    until(|| log(&session).contains("Ready"));
    let text = log(&session);
    let address = text
        .split("http://")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let revision = status(address)["revision"].as_u64().unwrap();
    fs::write(session.directory.path().join("src/lib.rs"), "broken").unwrap();
    until(|| {
        status(address)["error"]
            .as_str()
            .is_some_and(|e| e.contains("fixture compile failure"))
    });
    assert_eq!(status(address)["revision"], revision);
    fs::write(session.directory.path().join("src/lib.rs"), "fixed").unwrap();
    until(|| status(address)["revision"].as_u64().unwrap() > revision);
    assert!(status(address)["error"].is_null());
}

fn pids(session: &Session) -> Vec<i32> {
    fs::read_to_string(session.directory.path().join("build output/app-pids"))
        .unwrap_or_default()
        .lines()
        .map(|line| line.parse().unwrap())
        .collect()
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn desktop_retains_app_after_error_restarts_after_fix_and_cleans_up() {
    let mut session = session("desktop");
    until(|| !pids(&session).is_empty());
    let first = pids(&session)[0];
    fs::write(session.directory.path().join("src/lib.rs"), "broken").unwrap();
    until(|| log(&session).contains("fixture compile failure"));
    assert!(alive(first));
    fs::write(session.directory.path().join("src/lib.rs"), "fixed").unwrap();
    until(|| pids(&session).len() >= 2);
    let second = *pids(&session).last().unwrap();
    until(|| !alive(first));
    assert!(alive(second));
    unsafe {
        libc::kill(session.child.id() as i32, libc::SIGTERM);
    }
    until(|| session.child.try_wait().unwrap().is_some());
    assert!(!alive(second));
}
