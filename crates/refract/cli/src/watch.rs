//! Portable polling watcher: includes path dependencies, excludes generated output.
use crate::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

pub fn snapshot(roots: &[PathBuf], target_dir: &Path) -> Result<Snapshot> {
    let mut result = BTreeMap::new();
    for root in roots {
        visit(root, target_dir, &mut result)?;
    }
    Ok(result)
}

fn visit(path: &Path, target_dir: &Path, result: &mut Snapshot) -> Result<()> {
    if path.starts_with(target_dir) {
        return Ok(());
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if [
        "target",
        "dist",
        ".git",
        ".tools",
        "node_modules",
        ".DS_Store",
        ".refract",
    ]
    .contains(&name)
        || name.starts_with(".refract-build-")
        || name.ends_with('~')
        || name.ends_with(".swp")
        || (name == "pkg" && path.parent().is_some_and(|p| p.ends_with("public")))
    {
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() {
        if path.join(".refract-build.json").is_file() {
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            visit(&entry?.path(), target_dir, result)?;
        }
    } else if metadata.is_file() {
        result.insert(path.to_path_buf(), (metadata.modified()?, metadata.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_add_edit_delete_but_ignores_generated_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("public/pkg")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        let snap = || snapshot(&[root.to_path_buf()], &root.join("target")).unwrap();
        let empty = snap();
        fs::write(root.join("target/build"), "build").unwrap();
        fs::write(root.join("public/pkg/app.js"), "output").unwrap();
        for directory in ["dist/web", ".refract-build-staging", "custom-output"] {
            fs::create_dir_all(root.join(directory)).unwrap();
            if directory == "custom-output" {
                fs::write(root.join(directory).join(".refract-build.json"), "{}").unwrap();
            }
            fs::write(root.join(directory).join("app.js"), "output").unwrap();
        }
        assert_eq!(snap(), empty);
        fs::write(root.join("app.rs"), "first").unwrap();
        let added = snap();
        assert_ne!(added, empty);
        fs::write(root.join("app.rs"), "second revision").unwrap();
        assert_ne!(snap(), added);
        fs::remove_file(root.join("app.rs")).unwrap();
        assert_eq!(snap(), empty);
    }
}
