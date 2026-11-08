use crate::compile::{bindgen_tool, build};
use crate::process::{self, ManagedChild};
use crate::project::Project;
use crate::server::{BuildState, DevServer, SharedState};
use crate::{watch, DevOptions, Platform, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn run(options: DevOptions, stopped: Arc<AtomicBool>) -> Result<()> {
    if options.project.platform == Platform::Desktop && options.open {
        return Err("--open applies only to web development".into());
    }
    let mut project = Project::load(&options.project, &stopped)?;
    if options.project.platform == Platform::Web
        && !project.root.join("public/index.html").is_file()
    {
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
    let server = if options.project.platform == Platform::Web {
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
                    project = Project::load(&options.project, &stopped)?;
                    roots = project.watch_roots();
                }
                let output = build(&project, &options.project, false, &stopped)?;
                if options.project.platform == Platform::Web {
                    let version = output
                        .bindgen_version
                        .ok_or("the WASM build did not compile wasm-bindgen")?;
                    let bindgen = bindgen_tool(&project, &version, &options.project, &stopped)?;
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
