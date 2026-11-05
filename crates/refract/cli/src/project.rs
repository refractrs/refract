use crate::{process, DevOptions, Platform, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Deserialize)]
pub struct Target {
    pub name: String,
    pub kind: Vec<String>,
    pub crate_types: Vec<String>,
    #[serde(default, rename = "required-features")]
    pub required_features: Vec<String>,
}

#[derive(Clone, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: PathBuf,
    pub source: Option<String>,
    pub targets: Vec<Target>,
    pub features: std::collections::BTreeMap<String, serde_json::Value>,
    pub default_run: Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
    target_directory: PathBuf,
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
}

#[derive(Deserialize)]
struct ResolveNode {
    id: String,
    dependencies: Vec<String>,
}

pub struct Project {
    pub package: Package,
    pub packages: Vec<Package>,
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub target_dir: PathBuf,
    pub target: Target,
    pub features: Vec<String>,
    watched_packages: Vec<Package>,
}

impl Project {
    pub fn load(options: &DevOptions, stopped: &AtomicBool) -> Result<Self> {
        // Select the package without resolving unrelated workspace dependencies or
        // applying app features to a virtual workspace root.
        let mut command = Command::new("cargo");
        command.args(["metadata", "--format-version", "1", "--no-deps"]);
        if let Some(path) = &options.manifest_path {
            let path = path.canonicalize()?;
            command.current_dir(path.parent().unwrap());
            command.arg("--manifest-path").arg(path);
        }
        cargo_options(&mut command, options);
        let metadata = read_metadata(&mut command, stopped)?;
        let members: Vec<_> = metadata
            .packages
            .iter()
            .filter(|p| metadata.workspace_members.contains(&p.id))
            .collect();
        let manifest = options
            .manifest_path
            .as_deref()
            .map(Path::canonicalize)
            .transpose()?;
        let cwd = std::env::current_dir()?.canonicalize()?;
        let selected = if let Some(name) = &options.package {
            members.iter().copied().find(|p| &p.name == name)
        } else {
            let location = manifest.as_deref().and_then(Path::parent).unwrap_or(&cwd);
            members
                .iter()
                .copied()
                .filter(|p| location.starts_with(p.manifest_path.parent().unwrap()))
                .max_by_key(|p| p.manifest_path.components().count())
                .or_else(|| (members.len() == 1).then(|| members[0]))
        };
        let package = selected
            .ok_or_else(|| {
                format!(
                    "select an application with --package <name>; workspace packages: {}",
                    members
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
            .clone();
        let target = select_target(&package, options.platform, options.bin.as_deref())?;
        let mut features = options.features.clone();
        if options.platform == Platform::Desktop {
            if package.features.contains_key("desktop") {
                features.push("desktop".into());
            }
            features.extend(target.required_features.clone());
        }
        features.sort();
        features.dedup();
        let root = package.manifest_path.parent().unwrap().to_path_buf();
        let platform = if options.platform == Platform::Web {
            "wasm32-unknown-unknown".to_string()
        } else {
            let output = process::capture(
                Command::new("rustc").current_dir(&root).arg("-vV"),
                stopped,
                false,
                true,
            )?;
            output
                .stdout
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .ok_or("could not determine native Rust target")?
                .to_string()
        };
        let mut command = Command::new("cargo");
        command
            .current_dir(&root)
            .args(["metadata", "--format-version", "1"])
            .arg("--manifest-path")
            .arg(&package.manifest_path)
            .arg("--filter-platform")
            .arg(platform);
        cargo_options(&mut command, options);
        if !features.is_empty() {
            command.arg("--features").arg(features.join(","));
        }
        let metadata = read_metadata(&mut command, stopped)?;

        let mut reachable = std::collections::BTreeSet::new();
        let mut pending = vec![package.id.clone()];
        while let Some(id) = pending.pop() {
            if reachable.insert(id.clone()) {
                if let Some(node) = metadata
                    .resolve
                    .as_ref()
                    .and_then(|r| r.nodes.iter().find(|n| n.id == id))
                {
                    pending.extend(node.dependencies.iter().cloned());
                }
            }
        }
        let watched_packages = metadata
            .packages
            .iter()
            .filter(|p| reachable.contains(&p.id))
            .cloned()
            .collect();
        Ok(Self {
            package,
            packages: metadata.packages,
            root,
            workspace: metadata.workspace_root,
            target_dir: metadata.target_directory,
            target,
            features,
            watched_packages,
        })
    }

    pub fn watch_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<_> = self
            .watched_packages
            .iter()
            .filter(|p| p.source.is_none())
            .map(|p| p.manifest_path.parent().unwrap().to_path_buf())
            .collect();
        // Path dependencies can inherit workspace settings from a different checkout.
        let package_roots = roots.clone();
        for root in package_roots {
            for ancestor in root.ancestors() {
                roots.extend(
                    [
                        "Cargo.toml",
                        "Cargo.lock",
                        "rust-toolchain",
                        "rust-toolchain.toml",
                        ".cargo/config",
                        ".cargo/config.toml",
                    ]
                    .map(|name| ancestor.join(name)),
                );
            }
        }
        // Include workspace configuration without scanning unrelated workspace members.
        roots.extend([
            self.workspace.join("Cargo.toml"),
            self.workspace.join("Cargo.lock"),
            self.workspace.join(".cargo"),
        ]);
        roots.sort();
        roots.dedup();
        roots
    }
}

fn select_target(package: &Package, platform: Platform, binary: Option<&str>) -> Result<Target> {
    if platform == Platform::Web {
        if binary.is_some() {
            return Err("--bin applies only to --platform desktop".into());
        }
        return package
            .targets
            .iter()
            .find(|t| t.crate_types.iter().any(|kind| kind == "cdylib"))
            .cloned()
            .ok_or_else(|| {
                "web development requires a library with crate-type = [\"cdylib\", \"rlib\"]".into()
            });
    }
    let targets: Vec<_> = package
        .targets
        .iter()
        .filter(|t| t.kind.iter().any(|kind| kind == "bin"))
        .collect();
    let preferred = binary.or(package.default_run.as_deref());
    let selected = if let Some(name) = preferred {
        targets.iter().copied().find(|t| t.name == name)
    } else if targets.len() == 1 {
        Some(targets[0])
    } else {
        targets
            .iter()
            .copied()
            .find(|t| t.name == format!("{}-desktop", package.name))
    };
    selected.cloned().ok_or_else(|| {
        "select a desktop executable with --bin <name> (no unambiguous binary found)".into()
    })
}

pub fn cargo_options(command: &mut Command, options: &DevOptions) {
    if options.offline {
        command.arg("--offline");
    }
    if options.locked {
        command.arg("--locked");
    }
    if options.no_default_features {
        command.arg("--no-default-features");
    }
}

fn read_metadata(command: &mut Command, stopped: &AtomicBool) -> Result<Metadata> {
    let output = process::capture(command, stopped, false, true)?;
    if !output.success {
        return Err(format!("could not read Cargo project:\n{}", output.stderr).into());
    }
    Ok(serde_json::from_str(&output.stdout)?)
}
