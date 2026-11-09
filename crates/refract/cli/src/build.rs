//! One-shot builds, staged beside the destination to preserve the last good output.
use crate::{compile, process, project::Project, Platform, ProjectOptions, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const MARKER: &str = ".refract-build.json";

#[derive(clap::Args)]
pub struct BuildOptions {
    #[command(flatten)]
    project: ProjectOptions,
    /// Compile with Cargo's optimized release profile.
    #[arg(long)]
    release: bool,
    /// Output directory, relative to the app directory (default: dist/web or dist/desktop).
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Web URL prefix, e.g. /my-app/. Sets the entry HTML's base URL.
    #[arg(long, value_parser = base_path)]
    base_path: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Ownership {
    refract_build: u8,
    package: String,
    platform: String,
}

pub fn run(options: BuildOptions, stopped: &AtomicBool) -> Result<()> {
    if options.project.platform == Platform::Native && options.base_path.is_some() {
        return Err("--base-path applies only to web builds".into());
    }
    let project = Project::load(&options.project, stopped)?;
    let platform = match options.project.platform {
        Platform::Web => "web",
        Platform::Native => "desktop",
    };
    let ownership = Ownership {
        refract_build: 1,
        package: project.package.name.clone(),
        platform: platform.into(),
    };
    let output = destination(
        &project.root.join(
            options
                .out_dir
                .unwrap_or_else(|| PathBuf::from("dist").join(platform)),
        ),
    )?;
    protect_sources(&project, &output)?;
    check_output(&output, &ownership)?;
    let public = project.root.join("public");
    let html = if options.project.platform == Platform::Web {
        let html = fs::read_to_string(public.join("index.html"))
            .map_err(|e| format!("web builds require readable public/index.html: {e}"))?;
        Some(match &options.base_path {
            Some(base) => super::html::with_base_path(&html, base)?,
            None => html,
        })
    } else {
        None
    };

    let artifact = compile::build(&project, &options.project, options.release, stopped)?;
    let parent = output
        .parent()
        .ok_or("output must have a parent directory")?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".refract-build-")
        .tempdir_in(parent)?;
    let candidate = staging.path().join("output");
    fs::create_dir(&candidate)?;
    match options.project.platform {
        Platform::Web => {
            copy_tree(&public, &candidate, true)?;
            fs::write(candidate.join("index.html"), html.unwrap())?;
            let version = artifact
                .bindgen_version
                .ok_or("the WASM build did not compile wasm-bindgen")?;
            let bindgen = compile::bindgen_tool(&project, &version, &options.project, stopped)?;
            let pkg = candidate.join("pkg");
            fs::create_dir(&pkg)?;
            process::checked(
                Command::new(bindgen)
                    .args(["--target", "web", "--out-dir"])
                    .arg(&pkg)
                    .arg(&artifact.path),
                stopped,
            )?;
        }
        Platform::Native => {
            let name = artifact
                .path
                .file_name()
                .ok_or("missing executable filename")?;
            if name == MARKER || name == "assets" {
                return Err(
                    "desktop executable name conflicts with the distribution layout".into(),
                );
            }
            fs::copy(&artifact.path, candidate.join(name))?;
            let assets = project.root.join("assets");
            if assets.try_exists()? {
                copy_tree(&assets, &candidate.join("assets"), false)?;
            }
        }
    }
    fs::write(
        candidate.join(MARKER),
        serde_json::to_vec_pretty(&ownership)?,
    )?;
    if stopped.load(Ordering::Relaxed) {
        return Err("interrupted".into());
    }
    publish(staging, &output, &ownership)?;
    eprintln!("Built {} → {}", project.package.name, output.display());
    if options.project.platform == Platform::Native {
        eprintln!("Run the executable with this output directory as the working directory.");
    }
    Ok(())
}

fn base_path(value: &str) -> std::result::Result<String, String> {
    if !value.starts_with('/') || value.starts_with("//") {
        return Err("base path must start with a single '/', e.g. /my-app/".into());
    }
    let path = value.strip_suffix('/').unwrap_or(value);
    if !path.is_empty()
        && path[1..].split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
        })
    {
        return Err(
            "base path must contain URL segments using letters, digits, '-', '_', '.', or '~'"
                .into(),
        );
    }
    Ok(format!("{path}/"))
}

/// Resolve existing ancestors, including symlinks, before checking overlap.
fn destination(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!("output must not be a symlink: {}", path.display()).into());
            }
            Ok(path.canonicalize()?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("invalid output directory")?;
            let name = path.file_name().ok_or("invalid output directory")?;
            // Canonicalize the nearest existing ancestor without creating directories.
            let parent = if parent.exists() {
                parent.canonicalize()?
            } else {
                destination(parent)?
            };
            Ok(parent.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

fn protect_sources(project: &Project, output: &Path) -> Result<()> {
    let mut roots: Vec<_> = project
        .packages
        .iter()
        .filter(|p| p.source.is_none())
        .filter_map(|p| p.manifest_path.parent())
        .collect();
    roots.push(&project.workspace);
    for root in roots {
        let root = root.canonicalize()?;
        if root.starts_with(output) {
            return Err("output must not replace a package or workspace directory".into());
        }
        for name in ["src", "public", "assets", ".git", ".cargo"] {
            let path = root.join(name);
            let source = if path.exists() {
                path.canonicalize()?
            } else {
                destination(&path)?
            };
            if output.starts_with(&source) || source.starts_with(output) {
                return Err(
                    format!("output overlaps source directory {}", source.display()).into(),
                );
            }
        }
    }
    let target = if project.target_dir.exists() {
        project.target_dir.canonicalize()?
    } else {
        destination(&project.target_dir)?
    };
    if target.starts_with(output) {
        return Err("output must not replace Cargo's target directory".into());
    }
    Ok(())
}

fn check_output(output: &Path, ownership: &Ownership) -> Result<()> {
    match fs::symlink_metadata(output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("output must be a directory, not a file or symlink".into());
        }
        Ok(_) => {}
    }
    if fs::read_dir(output)?.next().is_none() {
        return Ok(());
    }
    let previous = fs::read(output.join(MARKER))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Ownership>(&bytes).ok());
    if previous.as_ref() != Some(ownership) {
        return Err(format!("refusing to replace {}: not an output directory for this app and platform; choose --out-dir", output.display()).into());
    }
    Ok(())
}

fn publish(staging: tempfile::TempDir, output: &Path, ownership: &Ownership) -> Result<()> {
    // Recheck after compilation, since the destination may have changed meanwhile.
    check_output(output, ownership)?;
    let backup = staging.path().join("previous");
    let had_previous = output.try_exists()?;
    if had_previous {
        fs::rename(output, &backup)?;
    }
    if let Err(error) = fs::rename(staging.path().join("output"), output) {
        if had_previous {
            if let Err(restore) = fs::rename(&backup, output) {
                let saved = staging.keep();
                return Err(format!("could not publish: {error}; could not restore: {restore}; previous output preserved in {}", saved.join("previous").display()).into());
            }
        }
        return Err(error.into());
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, public_root: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "asset directory must be a real directory: {}",
            source.display()
        )
        .into());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".DS_Store"
            || (public_root && (name == "pkg" || name == "__refract" || name == MARKER))
        {
            continue;
        }
        let kind = entry.file_type()?;
        let target = destination.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, false)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(format!(
                "assets must be regular files or directories; symlinks are not copied: {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_publication_restores_previous_output() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("dist");
        fs::create_dir(&output).unwrap();
        let ownership = Ownership {
            refract_build: 1,
            package: "app".into(),
            platform: "web".into(),
        };
        fs::write(output.join(MARKER), serde_json::to_vec(&ownership).unwrap()).unwrap();
        fs::write(output.join("index.html"), "previous build").unwrap();
        let staging = tempfile::tempdir_in(temp.path()).unwrap();
        // A missing candidate forces failure after moving the previous directory.
        assert!(publish(staging, &output, &ownership).is_err());
        assert_eq!(
            fs::read_to_string(output.join("index.html")).unwrap(),
            "previous build"
        );
    }
}
