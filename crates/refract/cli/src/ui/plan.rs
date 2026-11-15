use super::catalog;
use crate::Result;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Reject symlinks at every existing component, even when they point back into
/// the package. All source and destination paths use the same containment rule.
pub fn safe_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    catalog::relative(relative)?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(
                    format!("symlinks are not supported in UI paths: {}", path.display()).into(),
                )
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

struct Change {
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}
pub struct Plan {
    root: PathBuf,
    changes: BTreeMap<PathBuf, Change>,
}
impl Plan {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            changes: BTreeMap::new(),
        }
    }
    pub fn read(&self, relative: &Path) -> Result<Option<Vec<u8>>> {
        if let Some(change) = self.changes.get(relative) {
            return Ok(Some(change.after.clone()));
        }
        read_optional(&safe_path(&self.root, relative)?)
    }
    pub fn put(&mut self, relative: &Path, after: Vec<u8>) -> Result<()> {
        let path = safe_path(&self.root, relative)?;
        if let Some(change) = self.changes.get_mut(relative) {
            change.after = after;
        } else {
            self.changes.insert(
                relative.into(),
                Change {
                    before: read_optional(&path)?,
                    after,
                },
            );
        }
        Ok(())
    }
    pub fn finish(self, dry_run: bool, stopped: &AtomicBool) -> Result<()> {
        // Validate snapshots even for planned files whose final contents are
        // unchanged (notably the manifest used for source compatibility).
        if !dry_run {
            for (relative, change) in &self.changes {
                let path = safe_path(&self.root, relative)?;
                if read_optional(&path)? != change.before {
                    return Err(format!(
                        "{} changed during installation; retry",
                        relative.display()
                    )
                    .into());
                }
            }
        }
        let changes: Vec<_> = self
            .changes
            .into_iter()
            .filter(|(_, c)| c.before.as_ref() != Some(&c.after))
            .collect();
        for (relative, change) in &changes {
            println!(
                "  {} {}",
                if change.before.is_some() { "~" } else { "+" },
                relative.display()
            );
        }
        if dry_run {
            println!("Dry run: no project files changed.");
            return Ok(());
        }
        // Roll back any partial writes on an ordinary error.
        let mut applied = Vec::new();
        let mut directories = Vec::new();
        let result = (|| -> Result<()> {
            for (relative, change) in &changes {
                if stopped.load(Ordering::Relaxed) {
                    return Err("interrupted".into());
                }
                let path = safe_path(&self.root, relative)?;
                let mut absent = Vec::new();
                let mut parent = path.parent().unwrap();
                while !parent.exists() {
                    absent.push(parent.to_path_buf());
                    parent = parent.parent().unwrap();
                }
                for directory in absent.into_iter().rev() {
                    fs::create_dir(&directory)?;
                    directories.push(directory);
                }
                atomic_write(&path, &change.after)?;
                applied.push((path, &change.before));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            for (path, before) in applied.into_iter().rev() {
                let rollback = match before {
                    Some(bytes) => atomic_write(&path, bytes),
                    None => fs::remove_file(&path).map_err(Into::into),
                };
                if let Err(e) = rollback {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
            for directory in directories.into_iter().rev() {
                let _ = fs::remove_dir(directory);
            }
            if !failures.is_empty() {
                return Err(format!("{error}; rollback failed: {}", failures.join("; ")).into());
            }
            return Err(error);
        }
        if changes.is_empty() {
            println!("Already installed; no changes.");
        }
        Ok(())
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    if let Ok(metadata) = fs::metadata(path) {
        file.as_file().set_permissions(metadata.permissions())?;
    }
    file.write_all(bytes)?;
    file.flush()?;
    file.persist(path)?;
    Ok(())
}

/// Append declarations without reformatting user code. Ambiguous inline,
/// conditional, or redirected modules must be configured by the user instead.
pub fn modules(bytes: Option<Vec<u8>>, names: &[String]) -> Result<Vec<u8>> {
    let mut text = String::from_utf8(bytes.unwrap_or_default())?;
    let parsed = syn::parse_file(&text)?;
    let mut additions = Vec::new();
    for name in names {
        catalog::identifier(name)?;
        let existing: Vec<_> = parsed
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Mod(module) if module.ident == name => Some(module),
                _ => None,
            })
            .collect();
        if existing.len() > 1
            || existing.first().is_some_and(|m| {
                m.content.is_some()
                    || m.attrs.iter().any(|a| {
                        a.path().is_ident("path")
                            || a.path().is_ident("cfg")
                            || a.path().is_ident("cfg_attr")
                    })
            })
        {
            return Err(format!(
                "module {name} is inline, conditional, or redirected; use a separate UI directory"
            )
            .into());
        }
        if existing.is_empty() && !additions.contains(name) {
            additions.push(name.clone());
        }
    }
    if !additions.is_empty() && !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for name in additions {
        text.push_str(&format!("pub mod {name};\n"));
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_manifest_edits_abort_before_creating_components() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("Cargo.toml");
        fs::write(&manifest, "original").unwrap();
        let mut plan = Plan::new(directory.path());
        plan.put(Path::new("Cargo.toml"), b"original".to_vec())
            .unwrap();
        plan.put(Path::new("src/ui/button.rs"), b"new".to_vec())
            .unwrap();
        fs::write(&manifest, "user edit").unwrap();
        assert!(plan
            .finish(false, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("changed during installation"));
        assert_eq!(fs::read_to_string(manifest).unwrap(), "user edit");
        assert!(!directory.path().join("src").exists());
    }

    #[test]
    fn cancellation_leaves_the_project_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let mut plan = Plan::new(directory.path());
        plan.put(Path::new("src/ui/button.rs"), b"new".to_vec())
            .unwrap();
        assert!(plan.finish(false, &AtomicBool::new(true)).is_err());
        assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
    }
}
