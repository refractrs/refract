//! Shared Cargo compilation and version-matched browser tooling.
use crate::project::{cargo_options, Project};
use crate::{process, Platform, ProjectOptions, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

pub struct Artifact {
    pub path: PathBuf,
    pub bindgen_version: Option<String>,
}

pub fn build(
    project: &Project,
    options: &ProjectOptions,
    release: bool,
    stopped: &AtomicBool,
) -> Result<Artifact> {
    eprintln!("Building {}…", project.package.name);
    let mut command = Command::new("cargo");
    command
        .current_dir(&project.root)
        .args([
            "build",
            "--message-format=json-render-diagnostics",
            "--color",
            "never",
        ])
        .arg("--manifest-path")
        .arg(&project.package.manifest_path)
        .arg("--package")
        .arg(&project.package.name);
    cargo_options(&mut command, options);
    if release {
        command.arg("--release");
    }
    if !project.features.is_empty() {
        command.arg("--features").arg(project.features.join(","));
    }
    match options.platform {
        Platform::Web => {
            ensure_wasm_target(&project.root, options, stopped)?;
            command.args(["--lib", "--target", "wasm32-unknown-unknown"]);
        }
        Platform::Native => {
            // Native means the host even if Cargo config defaults to a WASM target.
            let host = process::capture(
                Command::new("rustc").current_dir(&project.root).arg("-vV"),
                stopped,
                false,
                true,
            )?;
            let host = host
                .stdout
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .ok_or("could not determine native Rust target")?;
            command
                .arg("--bin")
                .arg(&project.target.name)
                .arg("--target")
                .arg(host);
        }
    }
    let output = process::capture(&mut command, stopped, true, false)?;
    let messages: Vec<Value> = output
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if !output.success {
        let diagnostics = messages
            .iter()
            .filter_map(|message| message["message"]["rendered"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let error = format!("{diagnostics}\n{}", output.stderr);
        // Keep the overlay useful without retaining unbounded compiler output.
        return Err(error.chars().take(64 * 1024).collect::<String>().into());
    }
    artifact(&messages, project, options.platform)
}

fn artifact(messages: &[Value], project: &Project, platform: Platform) -> Result<Artifact> {
    let mut path = None;
    let mut bindgen_version = None;
    for message in messages {
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        let id = message["package_id"].as_str().unwrap_or_default();
        if let Some(package) = project
            .packages
            .iter()
            .find(|p| p.id == id && p.name == "wasm-bindgen")
        {
            bindgen_version = Some(package.version.clone());
        }
        if id != project.package.id || message["target"]["name"] != project.target.name {
            continue;
        }
        if platform == Platform::Native {
            if let Some(executable) = message["executable"].as_str() {
                path = Some(PathBuf::from(executable));
            }
        } else if let Some(filenames) = message["filenames"].as_array() {
            path = filenames
                .iter()
                .filter_map(Value::as_str)
                .find(|path| Path::new(path).extension().is_some_and(|ext| ext == "wasm"))
                .map(PathBuf::from);
        }
    }
    Ok(Artifact {
        path: path
            .ok_or("Cargo succeeded but did not report the requested application artifact")?,
        bindgen_version,
    })
}

fn ensure_wasm_target(root: &Path, options: &ProjectOptions, stopped: &AtomicBool) -> Result<()> {
    let output = process::capture(
        Command::new("rustc").current_dir(root).args([
            "--print",
            "target-libdir",
            "--target",
            "wasm32-unknown-unknown",
        ]),
        stopped,
        false,
        true,
    )?;
    if output.success
        && fs::read_dir(output.stdout.trim()).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("libstd-"))
        })
    {
        return Ok(());
    }
    if options.offline {
        return Err("WASM target missing; run `rustup target add wasm32-unknown-unknown` before using --offline".into());
    }
    eprintln!("Installing the Rust WASM target…");
    process::checked(
        Command::new("rustup")
            .current_dir(root)
            .args(["target", "add", "wasm32-unknown-unknown"]),
        stopped,
    )
}

pub fn bindgen_tool(
    project: &Project,
    version: &str,
    options: &ProjectOptions,
    stopped: &AtomicBool,
) -> Result<PathBuf> {
    let executable = format!("wasm-bindgen{}", std::env::consts::EXE_SUFFIX);
    if let Some(override_path) = std::env::var_os("REFRACT_WASM_BINDGEN") {
        let path = PathBuf::from(override_path);
        if tool_matches(&path, version, stopped) {
            return Ok(path);
        }
        return Err(format!("REFRACT_WASM_BINDGEN must point to wasm-bindgen {version}").into());
    }
    let tool_root = project
        .target_dir
        .join("refract-tools")
        .join(format!("wasm-bindgen-{version}"));
    let installed = tool_root.join("bin").join(&executable);
    let mut candidates = vec![
        installed.clone(),
        project.workspace.join(".tools/bin").join(&executable),
    ];
    // Local framework checkouts may already have a matching tool installation.
    for package in &project.packages {
        if package.name == "refract" && package.source.is_none() {
            if let Some(root) = package
                .manifest_path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
            {
                candidates.push(root.join(".tools/bin").join(&executable));
            }
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|path| path.join(&executable)));
    }
    for path in candidates {
        if path.is_file() && tool_matches(&path, version, stopped) {
            return Ok(path);
        }
    }
    if options.offline {
        return Err(format!("wasm-bindgen {version} not found; install it or set REFRACT_WASM_BINDGEN before using --offline").into());
    }
    eprintln!(
        "Installing wasm-bindgen {version} into {}…",
        tool_root.display()
    );
    let rustc = process::capture(
        Command::new("rustc").current_dir(&project.root).arg("-vV"),
        stopped,
        false,
        true,
    )?;
    let host = rustc
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("could not determine the native target for wasm-bindgen-cli")?;
    process::checked(
        Command::new("cargo")
            .current_dir(&project.root)
            .args([
                "install",
                "wasm-bindgen-cli",
                "--version",
                version,
                "--locked",
                "--root",
            ])
            .arg(&tool_root)
            .arg("--target")
            .arg(host),
        stopped,
    )?;
    if !tool_matches(&installed, version, stopped) {
        return Err("installed wasm-bindgen did not report the expected version".into());
    }
    Ok(installed)
}

fn tool_matches(path: &Path, version: &str, stopped: &AtomicBool) -> bool {
    process::capture(Command::new(path).arg("--version"), stopped, false, true).is_ok_and(
        |output| output.success && output.stdout.trim() == format!("wasm-bindgen {version}"),
    )
}
