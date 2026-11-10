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

fn fixture(platform: &str) -> tempfile::TempDir {
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
  printf '%s\n' "$@" > "$FIXTURE/build output/cargo-args"
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
if [ -f "$FIXTURE/bindgen-fail" ]; then echo 'fixture bindgen failure' >&2; exit 1; fi
cp "$FIXTURE/build output/fixture.wasm" "$4/fixture_bg.wasm"
echo 'export default () => {};' > "$4/fixture.js"
"#,
    );
    directory
}

fn fixture_command(directory: &tempfile::TempDir) -> Command {
    let root = directory.path();
    let path = std::env::join_paths(
        std::iter::once(root.join("tools"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_refract"));
    command
        .current_dir(root)
        .env("FIXTURE", root)
        .env("PATH", path)
        .env("REFRACT_WASM_BINDGEN", root.join("tools/wasm-bindgen"));
    command
}

fn session(platform: &str) -> Session {
    let directory = fixture(platform);
    let log = fs::File::create(directory.path().join("build output/dev.log")).unwrap();
    let child = fixture_command(&directory)
        .args(["dev", "--platform", platform, "--port", "0", "--offline"])
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

fn build(directory: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    fixture_command(directory)
        .args(["build", "--offline"])
        .args(args)
        .output()
        .unwrap()
}

fn succeeds(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn prerender_publishes_html_and_preserves_previous_build_on_failure() {
    let directory = fixture("web");
    let root = directory.path();
    let html = "<head></head><main><!--refract:app--></main>";
    fs::write(root.join("public/index.html"), html).unwrap();
    let wasm = wat::parse_str(
        r#"(module
        (memory (export "memory") 1)
        (data (i32.const 16) "\00<div>hi</div>")
        (func (export "__refract_prerender_v1") (result i64)
            i64.const 60129542160))"#,
    )
    .unwrap();
    fs::write(root.join("build output/fixture.wasm"), &wasm).unwrap();
    succeeds(build(
        &directory,
        &["--release", "--prerender", "--base-path", "/demo/"],
    ));
    let dist = root.join("dist/web");
    let published = fs::read_to_string(dist.join("index.html")).unwrap();
    assert!(published.contains("<base href=\"/demo/\">"));
    assert!(published.contains("<main><div>hi</div></main>"));
    assert_eq!(
        fs::read_to_string(dist.join("app.html")).unwrap(),
        "<div>hi</div>"
    );
    assert!(dist.join("pkg/fixture.js").exists());
    let module = fs::read_to_string(dist.join("app.prerender.mjs")).unwrap();
    assert!(module.contains("export default Object.freeze("));
    assert!(module.contains("\"version\":1"));
    assert!(dist.join("app.prerender.d.mts").is_file());
    assert_eq!(
        fs::read_to_string(root.join("public/index.html")).unwrap(),
        html
    );

    // Rendering, marker validation, and bindgen must all succeed before publication.
    fs::write(
        root.join("build output/fixture.wasm"),
        wat::parse_str("(module)").unwrap(),
    )
    .unwrap();
    let failed = build(&directory, &["--prerender"]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("missing prerender export"));
    assert_eq!(
        fs::read_to_string(dist.join("index.html")).unwrap(),
        published
    );
    fs::write(root.join("build output/fixture.wasm"), &wasm).unwrap();
    fs::write(root.join("public/index.html"), "<main></main>").unwrap();
    assert!(!build(&directory, &["--prerender"]).status.success());
    assert_eq!(
        fs::read_to_string(dist.join("index.html")).unwrap(),
        published
    );
    fs::write(root.join("public/index.html"), html).unwrap();
    fs::write(root.join("bindgen-fail"), "").unwrap();
    assert!(!build(&directory, &["--prerender"]).status.success());
    assert_eq!(
        fs::read_to_string(dist.join("index.html")).unwrap(),
        published
    );
    fs::remove_file(root.join("bindgen-fail")).unwrap();

    // Prerendering remains opt-in; a later plain build removes its stale fragment.
    succeeds(build(&directory, &[]));
    assert_eq!(fs::read_to_string(dist.join("index.html")).unwrap(), html);
    assert!(!dist.join("app.html").exists());
    assert!(!dist.join("app.prerender.mjs").exists());
    assert!(!dist.join("app.prerender.d.mts").exists());
}

#[test]
fn prerender_is_a_web_build_option_only() {
    let directory = fixture("desktop");
    let failure = build(&directory, &["--platform", "desktop", "--prerender"]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("web builds only"));
    assert!(!directory.path().join("build output/cargo-args").exists());
    let failure = fixture_command(&directory)
        .args(["dev", "--prerender"])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("unexpected argument '--prerender'"));
}

#[test]
fn web_distribution_is_complete_and_only_replaces_output_after_success() {
    let directory = fixture("web");
    let root = directory.path();
    let html = "<!doctype html><head><link href='./style.css'></head><body><script type='module'>import init from './pkg/fixture.js'; await init();</script></body>";
    fs::write(root.join("public/index.html"), html).unwrap();
    fs::write(root.join("public/style.css"), "body { color: red; }").unwrap();
    fs::create_dir_all(root.join("public/.well-known")).unwrap();
    fs::write(root.join("public/.well-known/example"), "static file").unwrap();
    fs::create_dir_all(root.join("public/pkg")).unwrap();
    fs::write(root.join("public/pkg/stale.js"), "stale development output").unwrap();
    let args = [
        "--release",
        "--locked",
        "--features",
        "example",
        "--no-default-features",
        "--base-path",
        "/demo",
        "--out-dir",
        "distribution files",
    ];
    succeeds(build(&directory, &args));
    let dist = root.join("distribution files");
    let output_html = fs::read_to_string(dist.join("index.html")).unwrap();
    assert!(output_html.contains("<base href=\"/demo/\">"));
    assert!(!output_html.contains("__refract"));
    assert_eq!(
        fs::read(dist.join("pkg/fixture_bg.wasm")).unwrap(),
        b"\0asm"
    );
    assert!(dist.join("pkg/fixture.js").is_file());
    assert!(!dist.join("pkg/stale.js").exists());
    assert!(dist.join(".well-known/example").is_file());
    assert!(dist.join("style.css").is_file());
    assert_eq!(
        fs::read_to_string(root.join("public/index.html")).unwrap(),
        html
    );
    let cargo_args = fs::read_to_string(root.join("build output/cargo-args")).unwrap();
    for arg in [
        "--release",
        "--locked",
        "--offline",
        "--features",
        "example",
        "--no-default-features",
        "--lib",
        "wasm32-unknown-unknown",
    ] {
        assert!(
            cargo_args.lines().any(|line| line == arg),
            "missing {arg}: {cargo_args}"
        );
    }

    fs::write(root.join("src/lib.rs"), "broken").unwrap();
    assert!(!build(&directory, &args).status.success());
    assert_eq!(
        fs::read_to_string(dist.join("index.html")).unwrap(),
        output_html
    );
    fs::write(root.join("src/lib.rs"), "fixed").unwrap();
    fs::write(root.join("bindgen-fail"), "").unwrap();
    assert!(!build(&directory, &args).status.success());
    assert_eq!(
        fs::read_to_string(dist.join("index.html")).unwrap(),
        output_html
    );
    fs::remove_file(root.join("bindgen-fail")).unwrap();

    fs::remove_file(root.join("public/style.css")).unwrap();
    succeeds(build(&directory, &args));
    assert!(
        !dist.join("style.css").exists(),
        "stale files must disappear on success"
    );
    assert!(!fs::read_dir(root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".refract-build-")));

    // Default output and unmodified relative URLs also support relocation.
    succeeds(build(&directory, &[]));
    assert_eq!(
        fs::read_to_string(root.join("dist/web/index.html")).unwrap(),
        html
    );
    let cargo_args = fs::read_to_string(root.join("build output/cargo-args")).unwrap();
    assert!(!cargo_args.lines().any(|line| line == "--release"));
}

#[test]
fn desktop_distribution_runs_from_its_own_directory_with_assets() {
    let directory = fixture("desktop");
    let root = directory.path();
    fs::create_dir_all(root.join("assets/nested")).unwrap();
    fs::write(root.join("assets/nested/message.txt"), "packaged asset").unwrap();
    executable(
        &root.join("build output/fixture"),
        "#!/bin/sh\ncat assets/nested/message.txt\n",
    );
    succeeds(build(
        &directory,
        &["--platform", "desktop", "--release", "--bin", "fixture"],
    ));
    let dist = root.join("dist/desktop");
    let output = Command::new(dist.join("fixture"))
        .current_dir(&dist)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"packaged asset");
    assert!(!dist.join("index.html").exists());
    assert!(!dist.join("pkg").exists());
    let args = fs::read_to_string(root.join("build output/cargo-args")).unwrap();
    assert!(args.contains("--release\n"));
    assert!(args.contains("--target\nfixture-native\n"));
    assert!(args.contains("--bin\nfixture\n"));
    assert!(!build(
        &directory,
        &["--platform", "desktop", "--base-path", "/app/"]
    )
    .status
    .success());
}

#[test]
fn build_rejects_unowned_outputs_source_overlap_and_unsafe_assets() {
    let directory = fixture("web");
    let root = directory.path();
    fs::create_dir(root.join("unrelated")).unwrap();
    fs::write(root.join("unrelated/keep.txt"), "keep me").unwrap();
    for destination in [
        ".",
        "public",
        "public/output",
        "src",
        "build output",
        "unrelated",
    ] {
        assert!(
            !build(&directory, &["--out-dir", destination])
                .status
                .success(),
            "{destination}"
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("unrelated/keep.txt")).unwrap(),
        "keep me"
    );
    assert!(!root.join("build output/cargo-args").exists());

    succeeds(build(&directory, &["--release"]));
    std::os::unix::fs::symlink(root.join("src/lib.rs"), root.join("public/link")).unwrap();
    let failure = build(&directory, &["--release"]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("symlinks are not copied"));
    assert!(root.join("dist/web/pkg/fixture.js").is_file());
    std::os::unix::fs::symlink(root.join("dist/web"), root.join("linked-output")).unwrap();
    assert!(!build(&directory, &["--out-dir", "linked-output"])
        .status
        .success());
}

#[test]
fn base_path_validation_does_not_silently_change_existing_html_bases() {
    let directory = fixture("web");
    for base in [
        "https://example.com/",
        "//example.com/",
        "/../app/",
        "/a//b",
        "/app?query",
        "/app\"bad",
    ] {
        assert!(
            !build(&directory, &["--base-path", base]).status.success(),
            "{base}"
        );
    }
    fs::write(
        directory.path().join("public/index.html"),
        "<head><base href='/existing/'></head>",
    )
    .unwrap();
    assert!(!build(&directory, &["--base-path", "/new/"])
        .status
        .success());
    assert!(!directory.path().join("build output/cargo-args").exists());
}

#[test]
fn symlinked_sources_can_build_but_cannot_be_used_as_output() {
    let directory = fixture("web");
    let root = directory.path();
    fs::rename(root.join("src"), root.join("shared-source")).unwrap();
    std::os::unix::fs::symlink(root.join("shared-source"), root.join("src")).unwrap();
    succeeds(build(&directory, &[]));
    assert!(!build(&directory, &["--out-dir", "shared-source/output"])
        .status
        .success());
    assert_eq!(
        fs::read_to_string(root.join("shared-source/lib.rs")).unwrap(),
        "valid"
    );
}
