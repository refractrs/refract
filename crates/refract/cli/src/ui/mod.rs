//! Copy declared component source into a Cargo package, preserving local edits.
mod catalog;
mod plan;
mod source;

use crate::{project, Result};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use toml_edit::{DocumentMut, Item as TomlItem, Value};

#[derive(Args)]
pub struct Options {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Copy components and their dependencies into the selected package.
    Add(Add),
    /// List components available from a source.
    List(List),
    /// Register reusable source names in Cargo.toml.
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
}
#[derive(Args)]
struct Project {
    #[arg(short, long)]
    package: Option<String>,
    #[arg(long)]
    manifest_path: Option<PathBuf>,
    /// Use cached Cargo packages and local component sources only.
    #[arg(long)]
    offline: bool,
}
#[derive(Args)]
struct SourceOptions {
    /// Local directory, @alias, github:owner/repo, or git:https://host/repo.
    #[arg(long)]
    from: Option<String>,
    /// Git branch, tag, or commit. Resolved commits are recorded in the UI lock.
    #[arg(long, conflicts_with = "version")]
    rev: Option<String>,
    /// Version requirement for the published refract-ui crate.
    #[arg(long)]
    version: Option<String>,
}
#[derive(Args)]
struct Add {
    /// Names, @source/name, or github:owner/repo/name. A repository alone opens a picker.
    components: Vec<String>,
    #[command(flatten)]
    project: Project,
    #[command(flatten)]
    source: SourceOptions,
    /// Print planned changes without modifying the project.
    #[arg(long)]
    dry_run: bool,
    /// Replace conflicting files from the same component source.
    #[arg(long)]
    overwrite: bool,
    /// Install every component in the selected source.
    #[arg(long)]
    all: bool,
    /// Require components and their dependencies to support this platform.
    #[arg(long, value_parser = ["web", "desktop"])]
    platform: Option<String>,
}
#[derive(Args)]
struct List {
    /// Optional registered source, such as @acme.
    source_name: Option<String>,
    #[command(flatten)]
    project: Project,
    #[command(flatten)]
    source: SourceOptions,
}
#[derive(Subcommand)]
enum SourceCommand {
    /// Save a source without installing any components.
    Add {
        name: String,
        source: String,
        #[arg(long)]
        rev: Option<String>,
        #[command(flatten)]
        project: Project,
    },
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Config {
    directory: Option<PathBuf>,
    entry: Option<PathBuf>,
    default_source: Option<String>,
    #[serde(default)]
    sources: BTreeMap<String, SourceConfig>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum SourceConfig {
    Address(String),
    Pinned { source: String, rev: Option<String> },
}
impl SourceConfig {
    fn parts(&self) -> (&str, Option<&str>) {
        match self {
            Self::Address(s) => (s, None),
            Self::Pinned { source, rev } => (source, rev.as_deref()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lock {
    schema: u32,
    items: BTreeMap<String, Installed>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    source: String,
    revision: String,
    license: String,
    requires: Vec<String>,
    files: BTreeMap<String, String>,
}
fn hash(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn config(package: &project::Package) -> Result<Config> {
    let value = &package.metadata["refract"]["ui"];
    Ok(if value.is_null() {
        Config::default()
    } else {
        serde_json::from_value(value.clone())?
    })
}
fn select(options: &Project, stopped: &AtomicBool) -> Result<project::Package> {
    Ok(project::select_package(
        options.package.as_deref(),
        options.manifest_path.as_deref(),
        options.offline,
        stopped,
    )?
    .0)
}

pub fn run(options: Options, stopped: &AtomicBool) -> Result<()> {
    match options.command {
        Command::Add(options) => add(options, stopped),
        Command::List(mut options) => {
            if let Some(name) = options.source_name {
                if options.source.from.is_some() {
                    return Err("choose either a source argument or --from".into());
                }
                options.source.from = Some(name);
            }
            let package = select(&options.project, stopped)?;
            let (source, catalog) = load_source(
                &package,
                &config(&package)?,
                &options.source,
                options.project.offline,
                stopped,
            )?;
            println!("{} ({})", catalog.name, source.revision);
            list(&catalog);
            Ok(())
        }
        Command::Source {
            command:
                SourceCommand::Add {
                    name,
                    source,
                    rev,
                    project,
                },
        } => {
            catalog::identifier(&name)?;
            let package = select(&project, stopped)?;
            let root = package.manifest_path.parent().unwrap();
            let source = if is_git(&source) {
                source::git_url(&source)?;
                source
            } else {
                if rev.is_some() {
                    return Err("--rev applies only to Git sources".into());
                }
                Path::new(&source)
                    .canonicalize()?
                    .to_string_lossy()
                    .into_owned()
            };
            let mut plan = plan::Plan::new(root);
            let bytes = plan
                .read(Path::new("Cargo.toml"))?
                .ok_or("missing Cargo.toml")?;
            let mut document = String::from_utf8(bytes)?.parse::<DocumentMut>()?;
            let mut table = toml_edit::InlineTable::new();
            table.insert("source", source.into());
            if let Some(rev) = rev {
                table.insert("rev", rev.into());
            }
            let slot = &mut document["package"]["metadata"]["refract"]["ui"]["sources"][&name];
            let value = TomlItem::Value(Value::InlineTable(table));
            if !slot.is_none() && slot.to_string() != value.to_string() {
                return Err(
                    format!("source {name} already exists; edit Cargo.toml to change it").into(),
                );
            }
            *slot = value;
            plan.put(Path::new("Cargo.toml"), document.to_string().into_bytes())?;
            plan.finish(false, stopped)
        }
    }
}
fn is_git(source: &str) -> bool {
    source.starts_with("github:") || source.starts_with("git:")
}
fn list(catalog: &catalog::Catalog) {
    if catalog.items.is_empty() {
        println!("  No components available.");
    }
    for (name, item) in &catalog.items {
        println!(
            "  {name:16} {} [{}]",
            item.description,
            item.platforms.join(", ")
        );
    }
}

fn load_source(
    package: &project::Package,
    config: &Config,
    options: &SourceOptions,
    offline: bool,
    stopped: &AtomicBool,
) -> Result<(source::Source, catalog::Catalog)> {
    let mut address = options.from.as_deref().or(config.default_source.as_deref());
    let mut revision = options.rev.as_deref();
    let mut configured = false;
    if let Some(name) = address {
        if let Some(alias) = config.sources.get(name.strip_prefix('@').unwrap_or(name)) {
            let (source, rev) = alias.parts();
            address = Some(source);
            revision = revision.or(rev);
            configured = true;
        } else if name.starts_with('@') {
            return Err(format!("unknown UI source: {name}").into());
        }
    }
    if address.is_some() && options.version.is_some() {
        return Err("--version applies only to the default published refract-ui source".into());
    }
    let (framework, source) = if let Some(address) = address {
        let source = if is_git(address) {
            source::Source::git(address, revision, offline, stopped)?
        } else {
            if revision.is_some() {
                return Err("--rev applies only to Git sources".into());
            }
            let path = Path::new(address);
            let path = if configured && path.is_relative() {
                package.manifest_path.parent().unwrap().join(path)
            } else {
                path.into()
            };
            source::Source::local(&path)?
        };
        let (framework, _) = source::resolve_framework(package, None, offline, stopped)?;
        (framework, source)
    } else {
        if revision.is_some() {
            return Err("--rev requires a Git source".into());
        }
        // Releases track the application's resolved framework version. An
        // explicit requirement can select another compatible UI release.
        let (framework, _) = source::resolve_framework(package, None, offline, stopped)?;
        let matching = format!("={framework}");
        let version = options.version.as_deref().unwrap_or(&matching);
        semver::VersionReq::parse(version)?;
        let (framework, source) =
            source::resolve_framework(package, Some(version), offline, stopped)?;
        (framework, source.unwrap())
    };
    let catalog = catalog::Catalog::parse(&String::from_utf8(
        source.read("refract-ui.toml", stopped)?,
    )?)?;
    if !semver::VersionReq::parse(&catalog.compatibility.refract)?.matches(&framework) {
        return Err(format!(
            "{} requires Refract {}, but the application resolves to {framework}",
            catalog.name, catalog.compatibility.refract
        )
        .into());
    }
    Ok((source, catalog))
}

/// Qualified requests all use one source per invocation. This keeps source
/// revisions and sibling dependency resolution unambiguous.
fn requests(components: &[String], source: &mut SourceOptions) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for component in components {
        let qualified = if let Some(value) = component.strip_prefix("github:") {
            let parts: Vec<_> = value.split('/').collect();
            if !(2..=3).contains(&parts.len()) {
                return Err("expected github:owner/repository[/component]".into());
            }
            Some((
                format!("github:{}/{}", parts[0], parts[1]),
                parts.get(2).copied(),
            ))
        } else if component.starts_with('@') {
            let (alias, name) = component
                .split_once('/')
                .map_or((component.as_str(), None), |(a, n)| (a, Some(n)));
            Some((alias.into(), name))
        } else {
            None
        };
        if let Some((address, name)) = qualified {
            if source.from.as_ref().is_some_and(|s| s != &address) {
                return Err("install components from one source at a time".into());
            }
            source.from = Some(address);
            if let Some(name) = name {
                names.push(name.into());
            }
        } else {
            names.push(component.clone());
        }
    }
    Ok(names)
}

fn add(mut options: Add, stopped: &AtomicBool) -> Result<()> {
    let mut names = requests(&options.components, &mut options.source)?;
    let package = select(&options.project, stopped)?;
    let config = config(&package)?;
    let root = package.manifest_path.parent().unwrap();
    let mut plan = plan::Plan::new(root);
    // Snapshot the manifest before source resolution so concurrent edits fail.
    let mut manifest = String::from_utf8(
        plan.read(Path::new("Cargo.toml"))?
            .ok_or("missing Cargo.toml")?,
    )?
    .parse::<DocumentMut>()?;
    plan.put(Path::new("Cargo.toml"), manifest.to_string().into_bytes())?;
    let (source, catalog) = load_source(
        &package,
        &config,
        &options.source,
        options.project.offline,
        stopped,
    )?;
    if catalog.items.is_empty() {
        return Err(
            "this UI source contains no components; choose another source with --from".into(),
        );
    }
    if options.all {
        names = catalog.items.keys().cloned().collect();
    }
    if names.is_empty() {
        list(&catalog);
        if !io::stdin().is_terminal() {
            return Err("specify component names or --all when input is not interactive".into());
        }
        print!("Components to add (space-separated): ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        names = input.split_whitespace().map(str::to_owned).collect();
        if names.is_empty() {
            return Err("no components selected".into());
        }
    }
    let names = catalog.resolve(&names)?;
    let entry = if let Some(entry) = config.entry {
        entry
    } else {
        let libraries: Vec<_> = package
            .targets
            .iter()
            .filter(|t| {
                t.kind
                    .iter()
                    .any(|k| k == "lib" || k == "cdylib" || k == "rlib")
            })
            .collect();
        let binaries: Vec<_> = package
            .targets
            .iter()
            .filter(|t| t.kind.iter().any(|k| k == "bin"))
            .collect();
        let target = if libraries.len() == 1 {
            libraries[0]
        } else if libraries.is_empty() && binaries.len() == 1 {
            binaries[0]
        } else {
            return Err(
                "set package.metadata.refract.ui.entry to the crate root that should expose ui"
                    .into(),
            );
        };
        target
            .src_path
            .strip_prefix(root)
            .map_err(|_| "crate entry must be inside the selected package")?
            .into()
    };
    catalog::relative(&entry)?;
    let directory = config
        .directory
        .unwrap_or_else(|| entry.parent().unwrap().join("ui"));
    catalog::relative(&directory)?;
    if directory.parent() != entry.parent() {
        return Err("UI directory must be next to the configured crate entry (for example src/ui beside src/lib.rs)".into());
    }
    let module = directory
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid UI directory")?
        .to_owned();
    catalog::identifier(&module)?;
    if plan.read(&directory.with_extension("rs"))?.is_some() {
        return Err(format!(
            "{} already defines the UI module; choose a separate directory",
            directory.with_extension("rs").display()
        )
        .into());
    }
    let lock_path = Path::new("refract-ui.lock");
    let mut lock: Lock = match plan.read(lock_path)? {
        Some(bytes) => toml::from_str(std::str::from_utf8(&bytes)?)?,
        None => Lock {
            schema: 1,
            items: BTreeMap::new(),
        },
    };
    if lock.schema != 1 {
        return Err("unsupported UI lock schema".into());
    }
    let mut modules = Vec::new();
    let mut destinations = BTreeMap::new();
    println!(
        "Adding {} from {} ({})",
        names.join(", "),
        source.id,
        source.revision
    );
    for name in &names {
        let item = &catalog.items[name];
        if let Some(platform) = &options.platform {
            if !item.platforms.contains(platform) {
                return Err(format!("{name} does not support {platform}").into());
            }
        }
        let key = format!("{}/{name}", catalog.name);
        if lock.items.get(&key).is_some_and(|i| i.source != source.id) {
            return Err(format!("{key} is already installed from another source").into());
        }
        let mut files = BTreeMap::new();
        for file in &item.files {
            let target = directory.join(&file.target);
            let target_key = target
                .to_str()
                .ok_or("non-UTF-8 UI destination")?
                .replace('\\', "/");
            if destinations.insert(target.clone(), key.clone()).is_some() {
                return Err(format!("multiple components write {}", target.display()).into());
            }
            if lock.items.iter().any(|(owner, installed)| {
                owner != &key && installed.files.contains_key(&target_key)
            }) {
                return Err(format!(
                    "{} belongs to another installed component",
                    target.display()
                )
                .into());
            }
            let bytes = source.read(&file.source, stopped)?;
            let digest = hash(&bytes);
            if Path::new(&file.target)
                .extension()
                .is_some_and(|e| e == "rs")
            {
                syn::parse_file(std::str::from_utf8(&bytes)?)?;
                let first = Path::new(&file.target)
                    .components()
                    .next()
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .unwrap();
                let module = first.strip_suffix(".rs").unwrap_or(first);
                catalog::identifier(module)?;
                if !modules.contains(&module.to_owned()) {
                    modules.push(module.to_owned());
                }
            }
            let existing = plan.read(&target)?;
            let unchanged_source =
                lock.items.get(&key).and_then(|i| i.files.get(&target_key)) == Some(&digest);
            if existing.as_ref().is_some_and(|b| b != &bytes) && !options.overwrite {
                if unchanged_source {
                    println!("  = {} (keeping local edits)", target.display());
                } else {
                    return Err(format!(
                        "{} already exists with different contents; inspect it or use --overwrite",
                        target.display()
                    )
                    .into());
                }
            } else {
                plan.put(&target, bytes)?;
            }
            files.insert(target_key, digest);
        }
        // Keep provenance for old files when a newer catalog drops a file. An
        // add operation never deletes source that the application may still use.
        if let Some(previous) = lock.items.get(&key) {
            for (path, hash) in &previous.files {
                files.entry(path.clone()).or_insert(hash.clone());
            }
        }
        merge_dependencies(&mut manifest, &package, &item.dependencies)?;
        lock.items.insert(
            key,
            Installed {
                source: source.id.clone(),
                revision: source.revision.clone(),
                license: catalog.license.clone(),
                requires: item.requires.clone(),
                files,
            },
        );
    }
    for module in &modules {
        let file = plan.read(&directory.join(format!("{module}.rs")))?;
        let nested = plan.read(&directory.join(module).join("mod.rs"))?;
        if file.is_some() == nested.is_some() {
            return Err(format!("component module {module} must have exactly one entry: {module}.rs or {module}/mod.rs").into());
        }
    }
    let mod_path = directory.join("mod.rs");
    plan.put(&mod_path, plan::modules(plan.read(&mod_path)?, &modules)?)?;
    let entry_bytes = plan
        .read(&entry)?
        .ok_or("configured crate entry does not exist")?;
    plan.put(&entry, plan::modules(Some(entry_bytes), &[module])?)?;
    let license_path = directory
        .join("licenses")
        .join(format!("{}.txt", catalog.name));
    let license = source.read(&catalog.license_file, stopped)?;
    if plan.read(&license_path)?.is_some_and(|old| old != license) && !options.overwrite {
        return Err("source license changed; inspect it before using --overwrite".into());
    }
    plan.put(&license_path, license)?;
    plan.put(Path::new("Cargo.toml"), manifest.to_string().into_bytes())?;
    plan.put(lock_path, toml::to_string_pretty(&lock)?.into_bytes())?;
    plan.finish(options.dry_run, stopped)
}

fn merge_dependencies(
    document: &mut DocumentMut,
    package: &project::Package,
    dependencies: &BTreeMap<String, catalog::Dependency>,
) -> Result<()> {
    for (name, required) in dependencies {
        // Existing workspace/path/git choices remain intact. Deliberately reject
        // differing requirements instead of guessing whether their ranges overlap.
        if let Some(existing) = package
            .dependencies
            .iter()
            .find(|d| d["name"] == *name && d["kind"].is_null() && d["target"].is_null())
        {
            if existing["rename"]
                .as_str()
                .is_some_and(|alias| alias != name)
                || existing["optional"] == true
            {
                return Err(format!(
                    "{name} must be an unconditional dependency under its original name"
                )
                .into());
            }
            let current = existing["req"]
                .as_str()
                .ok_or("missing dependency requirement")?;
            if semver::VersionReq::parse(current)? != semver::VersionReq::parse(&required.version)?
            {
                return Err(format!("dependency {name} requires {}, but the project declares {current}; reconcile Cargo.toml first", required.version).into());
            }
            if required.default_features && existing["uses_default_features"] == false {
                return Err(format!(
                    "{name} requires default features; reconcile Cargo.toml first"
                )
                .into());
            }
        }
        let slot = &mut document["dependencies"][name];
        if slot.is_none() {
            let mut value = toml_edit::InlineTable::new();
            value.insert("version", required.version.clone().into());
            if !required.default_features {
                value.insert("default-features", false.into());
            }
            *slot = TomlItem::Value(Value::InlineTable(value));
        } else if let Some(version) = slot.as_str() {
            let decor = slot.as_value().unwrap().decor().clone();
            let mut value = toml_edit::InlineTable::new();
            value.insert("version", version.into());
            let mut value = Value::InlineTable(value);
            *value.decor_mut() = decor;
            *slot = TomlItem::Value(value);
        }
        if required.default_features
            && slot.get("default-features").and_then(TomlItem::as_bool) == Some(false)
        {
            return Err(
                format!("components require conflicting default features for {name}").into(),
            );
        }
        if let Some(version) = slot.get("version").and_then(TomlItem::as_str) {
            if semver::VersionReq::parse(version)? != semver::VersionReq::parse(&required.version)?
            {
                return Err(format!("components require conflicting versions of {name}").into());
            }
        }
        if !required.features.is_empty() {
            if slot.get("features").is_none() {
                slot["features"] = toml_edit::value(toml_edit::Array::new());
            }
            let features = slot["features"]
                .as_array_mut()
                .ok_or("dependency features must be an array")?;
            for feature in &required.features {
                if !features.iter().any(|f| f.as_str() == Some(feature)) {
                    features.push(feature.as_str());
                }
            }
        }
    }
    Ok(())
}
