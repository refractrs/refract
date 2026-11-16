use crate::Result;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Catalog {
    pub schema: u32,
    pub name: String,
    pub license: String,
    pub license_file: String,
    pub compatibility: Compatibility,
    pub items: BTreeMap<String, Item>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub refract: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub description: String,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    pub files: Vec<File>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub source: String,
    pub target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Dependency {
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default = "enabled")]
    pub default_features: bool,
}
fn enabled() -> bool {
    true
}

pub fn identifier(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || syn::parse_str::<syn::Ident>(name).is_err()
    {
        return Err(format!("invalid Rust module name: {name}").into());
    }
    Ok(())
}

/// Validate a native filesystem path, including Windows path separators.
pub fn relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.components().any(|c| match c {
            Component::Normal(part) => part.to_string_lossy().contains(['\\', ':']),
            _ => true,
        })
    {
        return Err(format!(
            "expected a relative path without traversal: {}",
            path.display()
        )
        .into());
    }
    Ok(())
}

/// Catalog and Git paths use forward slashes on every host.
pub fn portable_relative(path: &Path) -> Result<()> {
    if path.to_string_lossy().contains('\\') {
        return Err(format!(
            "expected a relative path without traversal: {}",
            path.display()
        )
        .into());
    }
    relative(path)
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self> {
        let catalog: Self = toml::from_str(text)?;
        if catalog.schema != 1 {
            return Err("unsupported UI catalog schema (expected 1)".into());
        }
        if catalog.name.is_empty()
            || !catalog
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return Err("invalid catalog name".into());
        }
        if catalog.license.trim().is_empty() {
            return Err("catalog license is required".into());
        }
        portable_relative(Path::new(&catalog.license_file))?;
        semver::VersionReq::parse(&catalog.compatibility.refract)?;
        for (name, item) in &catalog.items {
            identifier(name)?;
            if item.files.is_empty() {
                return Err(format!("{name} has no files").into());
            }
            if item.platforms.is_empty()
                || item.platforms.iter().any(|p| p != "web" && p != "desktop")
            {
                return Err(format!("{name} must declare web and/or desktop support").into());
            }
            for file in &item.files {
                portable_relative(Path::new(&file.source))?;
                portable_relative(Path::new(&file.target))?;
                if file.target == "mod.rs" || file.target.starts_with("licenses/") {
                    return Err(format!("reserved UI destination: {}", file.target).into());
                }
            }
            for (name, dep) in &item.dependencies {
                if name.is_empty()
                    || !name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                {
                    return Err(format!("invalid Cargo dependency: {name}").into());
                }
                semver::VersionReq::parse(&dep.version)?;
            }
        }
        // Validate the entire graph, including items not selected for installation.
        catalog.resolve(&catalog.items.keys().cloned().collect::<Vec<_>>())?;
        Ok(catalog)
    }

    pub fn resolve(&self, requested: &[String]) -> Result<Vec<String>> {
        fn visit(
            catalog: &Catalog,
            name: &str,
            visiting: &mut BTreeSet<String>,
            done: &mut BTreeSet<String>,
            order: &mut Vec<String>,
        ) -> Result<()> {
            if done.contains(name) {
                return Ok(());
            }
            if !visiting.insert(name.into()) {
                return Err(format!("component dependency cycle at {name}").into());
            }
            let item = catalog
                .items
                .get(name)
                .ok_or_else(|| format!("unknown component: {name}"))?;
            for dependency in &item.requires {
                visit(catalog, dependency, visiting, done, order)?;
            }
            visiting.remove(name);
            done.insert(name.into());
            order.push(name.into());
            Ok(())
        }
        let mut order = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();
        for name in requested {
            visit(self, name, &mut visiting, &mut done, &mut order)?;
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_native_relative_paths_but_keeps_catalog_paths_portable() {
        relative(&Path::new("src").join("lib.rs")).unwrap();
        relative(&Path::new("src").join("ui").join("button.rs")).unwrap();
        portable_relative(Path::new("src/ui/button.rs")).unwrap();
        assert!(portable_relative(Path::new(r"src\ui\button.rs")).is_err());
        assert_eq!(relative(Path::new(r"src\lib.rs")).is_ok(), cfg!(windows));
    }

    #[test]
    fn rejects_unsafe_relative_paths_on_every_host() {
        for path in [
            "",
            ".",
            "../outside",
            "src/../outside",
            "/absolute",
            "C:/absolute",
            r"C:\absolute",
            "C:relative",
            r"\rooted",
            r"\\server\share",
            r"src\..\outside",
            "src/file:stream",
        ] {
            assert!(relative(Path::new(path)).is_err(), "{path:?}");
            assert!(portable_relative(Path::new(path)).is_err(), "{path:?}");
        }
    }
}
