use crate::process::{self, ManagedChild};
use crate::project::{cargo_options, Project};
use crate::server::{BuildState, DevServer, SharedState};
use crate::{watch, DevOptions, Platform, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn run(options: DevOptions, stopped: Arc<AtomicBool>) -> Result<()> {
    if options.platform == Platform::Desktop && options.open {
        return Err("--open applies only to web development".into());
    }
    let mut project = Project::load(&options, &stopped)?;
    if options.platform == Platform::Web && !project.root.join("public/index.html").is_file() {
        return Err(
            "web development requires public/index.html; use `refract new` for a starter project"
                .into(),
        );
    }
    let session = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    );
    let scratch = Scratch(
        project
            .target_dir
            .join("refract-dev")
            .join(&project.package.name)
            .join(&session),
    );
    fs::create_dir_all(&scratch.0)?;
    let state = Arc::new(Mutex::new(BuildState {
        session,
        revision: 0,
        building: false,
        error: None,
        pkg: None,
    }));
    let server = if options.platform == Platform::Web {
        let server = DevServer::start(project.root.join("public"), options.port, state.clone())?;
        eprintln!(
            "Refract {} → {}\nWatching sources; Ctrl+C stops the server. Rebuilds reset app state.",
            project.package.name, server.url
        );
        if options.open {
            open_browser(&server.url);
        }
        Some(server)
    } else {
        eprintln!("Refract {} → desktop\nWatching sources; Ctrl+C stops the app. Rebuilds reset app state.", project.package.name);
        None
    };
    let mut app: Option<ManagedChild> = None;
    let mut app_directory: Option<PathBuf> = None;
    let mut roots = project.watch_roots();
    let mut previous = watch::snapshot(&roots, &project.target_dir)?;
    let mut generation = 0;
    let mut rebuild = true;
    let mut reload_metadata = false;
    while !stopped.load(Ordering::Relaxed) {
        if rebuild {
            generation += 1;
            {
                let mut status = state.lock().unwrap();
                status.building = true;
                status.error = None;
            }
            let result = (|| -> Result<()> {
                if reload_metadata {
                    // Preserve the package selection when launched from a virtual workspace.
                    project = Project::load(&options, &stopped)?;
                    roots = project.watch_roots();
                }
                let output = build(&project, &options, &stopped)?;
                if options.platform == Platform::Web {
                    let version = output
                        .bindgen_version
                        .ok_or("the WASM build did not compile wasm-bindgen")?;
                    let bindgen = bindgen_tool(&project, &version, &options, &stopped)?;
                    let directory = scratch.0.join(format!("web-{generation}"));
                    fs::create_dir(&directory)?;
                    let mut command = Command::new(bindgen);
                    command
                        .arg("--target")
                        .arg("web")
                        .arg("--out-dir")
                        .arg(&directory)
                        .arg(&output.path);
                    process::checked(&mut command, &stopped)?;
                    let old = {
                        let mut status = state.lock().unwrap();
                        let old = status.pkg.replace(directory);
                        status.revision += 1;
                        old
                    };
                    if let Some(old) = old {
                        let _ = fs::remove_dir_all(old);
                    }
                    eprintln!("Ready — {}", server.as_ref().unwrap().url);
                } else {
                    // Run a copy so Cargo can replace its output while the old app lives.
                    let directory = scratch.0.join(format!("desktop-{generation}"));
                    fs::create_dir(&directory)?;
                    let executable = directory.join(
                        output
                            .path
                            .file_name()
                            .ok_or("missing executable filename")?,
                    );
                    fs::copy(output.path, &executable)?;
                    // A failed build leaves the old app alive; replace only after success.
                    drop(app.take());
                    if let Some(old) = app_directory.take() {
                        let _ = fs::remove_dir_all(old);
                    }
                    let mut command = Command::new(&executable);
                    command
                        .current_dir(&project.root)
                        .env("REFRACT_DIAGNOSTICS", "1");
                    app = Some(ManagedChild::spawn(&mut command)?);
                    app_directory = Some(directory);
                    eprintln!(
                        "Ready — {} (runtime diagnostics enabled)",
                        project.target.name
                    );
                }
                Ok(())
            })();
            finish_build(&state, result, &stopped);
            rebuild = false;
            reload_metadata = true;
        }
        if let Some(child) = app.as_mut() {
            if let Some(status) = child.try_wait()? {
                eprintln!("Desktop app exited ({status}); save a file to relaunch.");
                app = None;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
        let next = watch::snapshot(&roots, &project.target_dir)?;
        if next != previous {
            // Debounce editor saves; keep the pre-build snapshot so edits made during
            // compilation trigger another build instead of being silently swallowed.
            std::thread::sleep(Duration::from_millis(150));
            previous = watch::snapshot(&roots, &project.target_dir)?;
            rebuild = true;
        }
    }
    drop(app);
    drop(server);
    eprintln!("Stopped.");
    Ok(())
}

fn finish_build(state: &SharedState, result: Result<()>, stopped: &AtomicBool) {
    let mut status = state.lock().unwrap();
    status.building = false;
    if let Err(error) = result {
        if !stopped.load(Ordering::Relaxed) {
            let error = error.to_string();
            eprintln!("Build failed: {error}\nWatching for changes…");
            status.error = Some(error);
        }
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Artifact {
    path: PathBuf,
    bindgen_version: Option<String>,
}

fn build(project: &Project, options: &DevOptions, stopped: &AtomicBool) -> Result<Artifact> {
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
    if !project.features.is_empty() {
        command.arg("--features").arg(project.features.join(","));
    }
    match options.platform {
        Platform::Web => {
            ensure_wasm_target(&project.root, options, stopped)?;
            command.args(["--lib", "--target", "wasm32-unknown-unknown"]);
        }
        Platform::Desktop => {
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
        if platform == Platform::Desktop {
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

fn ensure_wasm_target(root: &Path, options: &DevOptions, stopped: &AtomicBool) -> Result<()> {
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

fn bindgen_tool(
    project: &Project,
    version: &str,
    options: &DevOptions,
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

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let command = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = Command::new("xdg-open").arg(url).spawn();
    match command {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("Could not open a browser ({error}); visit {url}"),
    }
}
