//! Exercise installation against real Cargo packages without network access.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(directory.path().join("Cargo.toml"), format!(
        "# Preserve my manifest\n[package]\nname = \"ui-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n[dependencies]\nrefract = {{ path = {:?} }}\n", repository().join("crates/refract"))).unwrap();
    fs::write(
        directory.path().join("src/lib.rs"),
        "//! Preserve crate documentation.\n#![allow(dead_code)]\n\npub fn existing() {}\n",
    )
    .unwrap();
    directory
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_refract"))
        .current_dir(root)
        .env("CARGO_NET_OFFLINE", "true")
        .args(args)
        .output()
        .unwrap()
}
fn succeeds(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn fails(output: Output, expected: &str) {
    assert!(
        !output.status.success(),
        "expected failure: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    files
}

#[test]
fn installs_compilable_source_and_preserves_local_edits() {
    let source = collection();
    let project = fixture();
    let root = project.path();
    succeeds(custom(root, source.path(), &["hello"]));
    let entry = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(entry.starts_with("//! Preserve crate documentation."));
    assert!(entry.contains("pub mod ui;"));
    assert!(fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .starts_with("# Preserve my manifest"));
    assert!(
        !root.join("Cargo.lock").exists(),
        "source resolution must not write the app lock"
    );
    assert!(root.join("src/ui/licenses/fixture.txt").is_file());
    let lock: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("refract-ui.lock")).unwrap()).unwrap();
    assert_eq!(lock["items"].as_table().unwrap().len(), 2);
    assert_eq!(
        lock["items"]["fixture/hello"]["files"]["src/ui/hello.rs"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let before = snapshot(root);
    succeeds(custom(root, source.path(), &["hello"]));
    assert_eq!(snapshot(root), before);
    let hello = root.join("src/ui/hello.rs");
    fs::write(
        &hello,
        format!(
            "{}\n// application customization\n",
            fs::read_to_string(&hello).unwrap()
        ),
    )
    .unwrap();
    let customized = snapshot(root);
    succeeds(custom(root, source.path(), &["hello"]));
    assert_eq!(snapshot(root), customized);
    let mut compile = Command::new("cargo");
    compile.current_dir(root).args(["check", "--offline"]).env(
        "CARGO_TARGET_DIR",
        repository().join("target/ui-install-tests"),
    );
    succeeds(compile.output().unwrap());
}

#[test]
fn dry_run_conflicts_and_unknown_items_leave_project_untouched() {
    let source = collection();
    let project = fixture();
    let root = project.path();
    let before = snapshot(root);
    let output = succeeds(custom(root, source.path(), &["hello", "--dry-run"]));
    assert!(output.contains("src/ui/hello.rs"));
    assert_eq!(snapshot(root), before);
    fails(
        custom(root, source.path(), &["missing"]),
        "unknown component",
    );
    assert_eq!(snapshot(root), before);
    fs::create_dir(root.join("src/ui")).unwrap();
    fs::write(root.join("src/ui/hello.rs"), "// keep my hello\n").unwrap();
    let before = snapshot(root);
    fails(
        custom(root, source.path(), &["hello"]),
        "different contents",
    );
    assert_eq!(snapshot(root), before);
    succeeds(custom(root, source.path(), &["hello", "--overwrite"]));
    assert!(root.join("src/ui/shared.rs").is_file());
}

fn collection() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("LICENSE"), "Fixture license\n").unwrap();
    fs::write(
        directory.path().join("hello.rs"),
        "pub fn hello() -> impl refract::View { refract::text(super::shared::LABEL) }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("shared.rs"),
        "pub const LABEL: &str = \"hello\";\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("refract-ui.toml"),
        r#"schema = 1
name = "fixture"
license = "MIT"
license-file = "LICENSE"
[compatibility]
refract = ">=0.1.8, <0.2.0"
[items.hello]
description = "Hello"
platforms = ["web"]
requires = ["shared"]
files = [{ source = "hello.rs", target = "hello.rs" }]
[items.shared]
description = "Shared fixture data"
platforms = ["web"]
files = [{ source = "shared.rs", target = "shared.rs" }]
"#,
    )
    .unwrap();
    directory
}
fn custom(root: &Path, source: &Path, args: &[&str]) -> Output {
    let mut command = vec!["ui", "add", "--offline", "--from", source.to_str().unwrap()];
    command.extend(args);
    cli(root, &command)
}
#[test]
fn registers_sources_and_rejects_cycles_traversal_and_platform_mismatch() {
    let project = fixture();
    let root = project.path();
    let source = collection();
    succeeds(cli(
        root,
        &[
            "ui",
            "source",
            "add",
            "acme",
            source.path().to_str().unwrap(),
            "--offline",
        ],
    ));
    assert!(succeeds(cli(root, &["ui", "list", "@acme", "--offline"])).contains("Hello [web]"));
    let before = snapshot(root);
    fails(
        cli(
            root,
            &[
                "ui",
                "add",
                "@acme/hello",
                "--platform",
                "desktop",
                "--offline",
            ],
        ),
        "does not support desktop",
    );
    assert_eq!(snapshot(root), before);
    let manifest = source.path().join("refract-ui.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        original.replace("target = \"hello.rs\"", "target = \"../escape.rs\""),
    )
    .unwrap();
    fails(
        custom(root, source.path(), &["hello"]),
        "relative path without traversal",
    );
    assert_eq!(snapshot(root), before);
    fs::write(
        &manifest,
        original.replace("requires = [\"shared\"]", "requires = [\"hello\"]"),
    )
    .unwrap();
    fails(custom(root, source.path(), &["hello"]), "dependency cycle");
    assert_eq!(snapshot(root), before);
    fs::write(&manifest, original).unwrap();
    succeeds(cli(root, &["ui", "add", "@acme/hello", "--offline"]));
    assert!(root.join("src/ui/hello.rs").is_file());
}

#[test]
fn merges_cargo_features_preserving_comments_and_rejects_version_conflicts() {
    let project = fixture();
    let root = project.path();
    let source = collection();
    let source_manifest = source.path().join("refract-ui.toml");
    let mut text = fs::read_to_string(&source_manifest).unwrap();
    text.push_str("[items.hello.dependencies.serde]\nversion = \"1\"\nfeatures = [\"derive\"]\n");
    fs::write(&source_manifest, &text).unwrap();
    let path = root.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{manifest}serde = \"1\" # retain this comment\n"),
    )
    .unwrap();
    succeeds(custom(root, source.path(), &["hello"]));
    let manifest = fs::read_to_string(&path).unwrap();
    assert!(manifest.contains("retain this comment"));
    let parsed: toml::Value = toml::from_str(&manifest).unwrap();
    assert_eq!(
        parsed["dependencies"]["serde"]["features"][0].as_str(),
        Some("derive")
    );
    let before = snapshot(root);
    fs::write(
        &source_manifest,
        text.replace("version = \"1\"", "version = \"2\""),
    )
    .unwrap();
    fails(
        custom(root, source.path(), &["hello"]),
        "reconcile Cargo.toml",
    );
    assert_eq!(snapshot(root), before);
}

#[test]
fn selects_workspace_package_and_custom_module_directory() {
    let source = collection();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"app\", \"other\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    for name in ["app", "other"] {
        fs::create_dir_all(root.join(name).join("src")).unwrap();
        fs::write(root.join(name).join("Cargo.toml"), format!("[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2021\"\n[dependencies]\nrefract = {{ path = {:?} }}\n[package.metadata.refract.ui]\ndirectory = \"src/components\"\n", repository().join("crates/refract"))).unwrap();
        fs::write(root.join(name).join("src/main.rs"), "fn main() {}\n").unwrap();
    }
    fails(
        custom(root, source.path(), &["hello"]),
        "select an application",
    );
    succeeds(custom(root, source.path(), &["hello", "-p", "app"]));
    assert!(root.join("app/src/components/hello.rs").is_file());
    assert!(fs::read_to_string(root.join("app/src/main.rs"))
        .unwrap()
        .contains("pub mod components;"));
    assert!(!root.join("other/src/components").exists());
    assert!(!root.join("Cargo.lock").exists());
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_sources_and_destinations() {
    use std::os::unix::fs::symlink;
    let project = fixture();
    let root = project.path();
    let source = collection();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.join("src/ui")).unwrap();
    fails(
        custom(root, source.path(), &["hello"]),
        "symlinks are not supported",
    );
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    fs::remove_file(root.join("src/ui")).unwrap();
    fs::remove_file(source.path().join("hello.rs")).unwrap();
    fs::write(outside.path().join("secret.rs"), "pub fn secret() {}\n").unwrap();
    symlink(
        outside.path().join("secret.rs"),
        source.path().join("hello.rs"),
    )
    .unwrap();
    fails(
        custom(root, source.path(), &["hello"]),
        "symlinks are not supported",
    );
    assert!(!root.join("src/ui").exists());
}

#[test]
fn git_sources_install_a_pinned_snapshot_without_checking_out_files() {
    let project = fixture();
    let source = collection();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=UI Test",
            "-c",
            "user.email=ui@example.test",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
        vec!["tag", "v1"],
    ] {
        succeeds(
            Command::new("git")
                .current_dir(source.path())
                .args(args)
                .output()
                .unwrap(),
        );
    }
    let revision = succeeds(
        Command::new("git")
            .current_dir(source.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap(),
    );
    // Redirect the test HTTPS remote locally, using ordinary Git configuration.
    // No network or process-global environment changes are needed.
    let output = Command::new(env!("CARGO_BIN_EXE_refract"))
        .current_dir(project.path())
        .env("CARGO_NET_OFFLINE", "true")
        .env("GIT_CONFIG_COUNT", "1")
        .env(
            "GIT_CONFIG_KEY_0",
            format!("url.{}.insteadOf", source.path().display()),
        )
        .env(
            "GIT_CONFIG_VALUE_0",
            "https://github.com/test/collection.git",
        )
        .args(["ui", "add", "github:test/collection/hello", "--rev", "v1"])
        .output()
        .unwrap();
    succeeds(output);
    let lock = fs::read_to_string(project.path().join("refract-ui.lock")).unwrap();
    assert!(lock.contains(revision.trim()));
    assert_eq!(
        fs::read(project.path().join("src/ui/hello.rs")).unwrap(),
        fs::read(source.path().join("hello.rs")).unwrap()
    );
}

fn patch_default_source(root: &Path, source: &Path) {
    fs::create_dir(root.join(".cargo")).unwrap();
    fs::write(
        root.join(".cargo/config.toml"),
        format!(
            "[patch.crates-io]\nrefract-ui = {{ path = {:?} }}\nrefract = {{ path = {:?} }}\n",
            source,
            repository().join("crates/refract")
        ),
    )
    .unwrap();
}

#[test]
fn default_crate_source_uses_the_matching_framework_release() {
    let project = fixture();
    let root = project.path();
    let source = collection();
    // A synthetic published crate keeps installation coverage independent of
    // the contents of the default collection.
    fs::write(source.path().join("Cargo.toml"), format!(
        "[package]\nname = \"refract-ui\"\nversion = {:?}\nedition = \"2021\"\n[workspace]\n[lib]\npath = \"lib.rs\"\n",
        env!("CARGO_PKG_VERSION"),
    )).unwrap();
    fs::write(source.path().join("lib.rs"), "").unwrap();
    patch_default_source(root, source.path());
    succeeds(cli(root, &["ui", "add", "hello", "--offline"]));
    let lock: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("refract-ui.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["items"]["fixture/hello"]["source"].as_str(),
        Some("crate:refract-ui")
    );
    assert_eq!(
        lock["items"]["fixture/hello"]["revision"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(!root.join("Cargo.lock").exists());
    assert!(!fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .contains("refract-ui"));
}

#[test]
fn empty_default_collection_lists_and_rejects_installation_without_edits() {
    let project = fixture();
    let root = project.path();
    patch_default_source(root, &repository().join("crates/ui"));
    let before = snapshot(root);
    let listed = succeeds(cli(root, &["ui", "list", "--offline"]));
    assert!(listed.contains("refract-ui"));
    assert!(listed.contains("No components available"));
    for selection in ["button", "--all"] {
        fails(
            cli(root, &["ui", "add", selection, "--offline"]),
            "contains no components",
        );
    }
    assert_eq!(snapshot(root), before);
}

#[test]
fn preserves_workspace_dependencies_and_rejects_ambiguous_modules() {
    let project = fixture();
    let root = project.path();
    let source = collection();
    let path = root.join("Cargo.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!(
            "{original}serde = {{ workspace = true }}\n[workspace.dependencies]\nserde = \"1\"\n"
        ),
    )
    .unwrap();
    let path = source.path().join("refract-ui.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{original}[items.hello.dependencies.serde]\nversion = \"1\"\nfeatures = [\"derive\"]\n")).unwrap();
    let entry = root.join("src/lib.rs");
    fs::write(&entry, "#[cfg(test)]\nmod ui;\n").unwrap();
    let before = snapshot(root);
    fails(
        custom(root, source.path(), &["hello"]),
        "inline, conditional, or redirected",
    );
    assert_eq!(snapshot(root), before);
    fs::write(&entry, "// mod ui; is only a comment\n").unwrap();
    succeeds(custom(root, source.path(), &["hello"]));
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    assert_eq!(
        manifest["dependencies"]["serde"]["workspace"].as_bool(),
        Some(true)
    );
    assert_eq!(
        manifest["dependencies"]["serde"]["features"][0].as_str(),
        Some("derive")
    );
    assert_eq!(
        manifest["workspace"]["dependencies"]["serde"].as_str(),
        Some("1")
    );
}
