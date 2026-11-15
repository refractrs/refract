use super::catalog;
use crate::{process, project::Package, Result};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct Source {
    pub id: String,
    pub revision: String,
    root: PathBuf,
    git: bool,
    _temporary: Option<tempfile::TempDir>,
}

impl Source {
    pub fn local(path: &Path) -> Result<Self> {
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err("component source must be a directory".into());
        }
        Ok(Self {
            id: format!("path:{}", root.display()),
            revision: "local".into(),
            root,
            git: false,
            _temporary: None,
        })
    }

    pub fn git(
        address: &str,
        revision: Option<&str>,
        offline: bool,
        stopped: &AtomicBool,
    ) -> Result<Self> {
        if offline {
            return Err(
                "Git sources require network access; use a local checkout with --offline".into(),
            );
        }
        let url = git_url(address)?;
        let temporary = tempfile::tempdir()?;
        process::checked(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(temporary.path()),
            stopped,
        )?;
        // Fetch objects without checking out files: source attributes, filters,
        // hooks, and submodules never execute during installation.
        process::checked(
            Command::new("git").current_dir(temporary.path()).args([
                "fetch",
                "--quiet",
                "--depth=1",
                "--no-tags",
                "--",
                &url,
                revision.unwrap_or("HEAD"),
            ]),
            stopped,
        )?;
        let output = process::capture(
            Command::new("git")
                .current_dir(temporary.path())
                .args(["rev-parse", "FETCH_HEAD"]),
            stopped,
            false,
            true,
        )?;
        if !output.success {
            return Err("could not resolve source commit".into());
        }
        Ok(Self {
            id: url,
            revision: output.stdout.trim().into(),
            root: temporary.path().into(),
            git: true,
            _temporary: Some(temporary),
        })
    }

    pub fn read(&self, path: &str, stopped: &AtomicBool) -> Result<Vec<u8>> {
        catalog::relative(Path::new(path))?;
        if stopped.load(Ordering::Relaxed) {
            return Err("interrupted".into());
        }
        if self.git {
            let tree = process::capture(
                Command::new("git").current_dir(&self.root).args([
                    "ls-tree",
                    &self.revision,
                    "--",
                    path,
                ]),
                stopped,
                false,
                true,
            )?;
            if !tree.success
                || !(tree.stdout.starts_with("100644 blob ")
                    || tree.stdout.starts_with("100755 blob "))
            {
                return Err(
                    format!("source file is missing or is not a regular file: {path}").into(),
                );
            }
            // cat-file reads a blob verbatim, including binary assets. It does
            // not invoke filters, pagers, external diffs, or checkout hooks.
            let output = Command::new("git")
                .current_dir(&self.root)
                .args(["cat-file", "blob", &format!("{}:{path}", self.revision)])
                .output()?;
            if !output.status.success() {
                return Err(format!("could not read source file: {path}").into());
            }
            Ok(output.stdout)
        } else {
            super::plan::safe_path(&self.root, Path::new(path))?;
            Ok(fs::read(self.root.join(path))?)
        }
    }
}

pub fn git_url(address: &str) -> Result<String> {
    if let Some(path) = address.strip_prefix("github:") {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|p| {
                p.is_empty()
                    || p.starts_with('.')
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
            })
        {
            return Err("expected github:owner/repository".into());
        }
        return Ok(format!("https://github.com/{path}.git"));
    }
    if let Some(url) = address.strip_prefix("git:") {
        if url.starts_with("https://") && !url[8..].contains(['@', '?', '#']) {
            return Ok(url.into());
        }
        return Err(
            "Git sources use git:https://host/repository without embedded credentials".into(),
        );
    }
    Err("expected github:owner/repository or git:https://host/repository".into())
}

/// Resolve only the framework and optional UI crate in a temporary Cargo
/// project. This downloads source without building code or touching app locks.
pub fn resolve_framework(
    package: &Package,
    ui_version: Option<&str>,
    offline: bool,
    stopped: &AtomicBool,
) -> Result<(semver::Version, Option<Source>)> {
    let dependency = package
        .dependencies
        .iter()
        .find(|d| d["name"] == "refract" && d["kind"].is_null() && d["target"].is_null())
        .ok_or("UI installation requires a direct, unconditional refract dependency")?;
    if dependency["rename"]
        .as_str()
        .is_some_and(|s| s != "refract")
        || dependency["optional"] == true
    {
        return Err(
            "UI component sources require a non-optional framework dependency named refract".into(),
        );
    }
    let mut framework = toml::map::Map::new();
    framework.insert(
        "version".into(),
        dependency["req"]
            .as_str()
            .ok_or("missing refract version")?
            .into(),
    );
    framework.insert("default-features".into(), false.into());
    if let Some(path) = dependency["path"].as_str() {
        framework.insert("path".into(), path.into());
    } else if dependency["source"].as_str()
        != Some("registry+https://github.com/rust-lang/crates.io-index")
    {
        return Err(
            "UI installation currently supports path and crates.io refract dependencies".into(),
        );
    } else {
        // Honor an existing application lock rather than resolving a newer
        // framework patch in the isolated source-download project.
        for directory in package.manifest_path.parent().unwrap().ancestors() {
            let lock = directory.join("Cargo.lock");
            if lock.is_file() {
                let lock: toml::Value = toml::from_str(&fs::read_to_string(lock)?)?;
                let requirement = semver::VersionReq::parse(dependency["req"].as_str().unwrap())?;
                let matches: Vec<_> = lock
                    .get("package")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|p| {
                        p.get("name").and_then(toml::Value::as_str) == Some("refract")
                            && p.get("source").and_then(toml::Value::as_str)
                                == dependency["source"].as_str()
                    })
                    .filter_map(|p| p.get("version").and_then(toml::Value::as_str))
                    .filter(|v| semver::Version::parse(v).is_ok_and(|v| requirement.matches(&v)))
                    .collect();
                if matches.len() > 1 {
                    return Err("multiple locked Refract versions match the project; narrow its dependency requirement before installing UI".into());
                }
                if let Some(version) = matches.first() {
                    framework.insert("version".into(), format!("={version}").into());
                }
                break;
            }
            // A standalone nested project must not inherit an unrelated lock
            // from a workspace outside its own Cargo boundary.
            let manifest = directory.join("Cargo.toml");
            if manifest.is_file() {
                let manifest: toml::Value = toml::from_str(&fs::read_to_string(manifest)?)?;
                if manifest.get("workspace").is_some() {
                    break;
                }
            }
        }
    }
    let mut deps = toml::map::Map::new();
    deps.insert("refract".into(), toml::Value::Table(framework));
    if let Some(version) = ui_version {
        deps.insert("refract-ui".into(), version.into());
    }
    let temporary = tempfile::tempdir()?;
    fs::write(temporary.path().join("lib.rs"), "")?;
    fs::write(temporary.path().join("Cargo.toml"), format!(
        "[package]\nname = \"refract-ui-resolve\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n[lib]\npath = \"lib.rs\"\n{}", toml::to_string(&std::collections::BTreeMap::from([("dependencies", toml::Value::Table(deps))]))?))?;
    let mut command = Command::new("cargo");
    command
        .current_dir(package.manifest_path.parent().unwrap())
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(temporary.path().join("Cargo.toml"));
    if offline {
        command.arg("--offline");
    }
    let output = process::capture(&mut command, stopped, false, true)?;
    if !output.success {
        return Err(format!(
            "could not resolve UI source dependencies:\n{}",
            output.stderr
        )
        .into());
    }
    let metadata: Value = serde_json::from_str(&output.stdout)?;
    let root = &metadata["resolve"]["root"];
    let node = metadata["resolve"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|n| &n["id"] == root))
        .ok_or("missing source resolution root")?;
    let find = |name: &str| -> Result<&Value> {
        let edge = node["deps"]
            .as_array()
            .and_then(|deps| deps.iter().find(|d| d["name"] == name.replace('-', "_")))
            .ok_or("missing resolved dependency")?;
        metadata["packages"]
            .as_array()
            .and_then(|packages| packages.iter().find(|p| p["id"] == edge["pkg"]))
            .ok_or_else(|| "missing resolved package".into())
    };
    let framework = semver::Version::parse(
        find("refract")?["version"]
            .as_str()
            .ok_or("missing framework version")?,
    )?;
    let source = if ui_version.is_some() {
        let package = find("refract-ui")?;
        let mut source = Source::local(
            Path::new(
                package["manifest_path"]
                    .as_str()
                    .ok_or("missing UI source path")?,
            )
            .parent()
            .unwrap(),
        )?;
        source.id = "crate:refract-ui".into();
        source.revision = package["version"]
            .as_str()
            .ok_or("missing UI source version")?
            .into();
        Some(source)
    } else {
        None
    };
    Ok((framework, source))
}
