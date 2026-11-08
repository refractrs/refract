use crate::Result;
use std::fs;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct NewOptions {
    /// New directory to create; existing directories are never overwritten.
    pub path: PathBuf,
    /// Package name (defaults to the new directory name).
    #[arg(long)]
    pub name: Option<String>,
    /// Refract repository containing crates/refract, crates/web, and crates/desktop.
    /// Use local path dependencies instead of the published CLI's release version.
    #[arg(long)]
    pub framework_path: Option<PathBuf>,
}

pub fn create(options: NewOptions) -> Result<()> {
    let name = options
        .name
        .as_deref()
        .or_else(|| options.path.file_name().and_then(|n| n.to_str()))
        .ok_or("provide --name with a valid Rust package name")?;
    validate_name(name)?;
    if options.path.exists() {
        return Err(format!(
            "{} already exists; choose a new directory",
            options.path.display()
        )
        .into());
    }
    let framework = options
        .framework_path
        .map(|path| path.canonicalize())
        .transpose()?;
    let (rust_version, dependencies) = if let Some(framework) = &framework {
        for subdir in ["refract", "web", "desktop"] {
            if !framework
                .join("crates")
                .join(subdir)
                .join("Cargo.toml")
                .is_file()
            {
                return Err(format!(
                    "{} is not a Refract checkout (missing crates/{subdir}/Cargo.toml)",
                    framework.display()
                )
                .into());
            }
        }
        let workspace: toml::Value =
            toml::from_str(&fs::read_to_string(framework.join("Cargo.toml"))?)?;
        let rust_version = workspace
            .get("workspace")
            .and_then(|v| v.get("package"))
            .and_then(|v| v.get("rust-version"))
            .and_then(toml::Value::as_str)
            .ok_or("missing framework rust-version")?
            .to_owned();
        let dependencies = workspace
            .get("workspace")
            .and_then(|v| v.get("dependencies"))
            .ok_or("missing framework dependencies")?
            .clone();
        (rust_version, dependencies)
    } else {
        let manifest: toml::Value = toml::from_str(include_str!("../../Cargo.toml"))?;
        (
            env!("CARGO_PKG_RUST_VERSION").to_owned(),
            manifest["package"]["metadata"]["refract"]["scaffold"].clone(),
        )
    };
    let version = |name: &str| -> Result<String> {
        let value = dependencies
            .get(name)
            .ok_or_else(|| format!("missing scaffold {name} version"))?;
        value
            .as_str()
            .or_else(|| value.get("version").and_then(toml::Value::as_str))
            .map(quote)
            .ok_or_else(|| format!("missing scaffold {name} version").into())
    };
    let crate_name = name.replace('-', "_");
    let dependency = |part: &str| match &framework {
        Some(framework) => format!(
            "path = {}",
            quote(&framework.join("crates").join(part).to_string_lossy())
        ),
        None => format!("version = {}", quote(env!("CARGO_PKG_VERSION"))),
    };
    let manifest = format!(
        r#"[package]
name = {name}
version = "0.1.0"
edition = "2021"
rust-version = {rust_version}
publish = false

# A standalone workspace allows creation inside another checkout.
[workspace]
resolver = "2"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
refract = {{ {refract} }}

[target.'cfg(target_arch = "wasm32")'.dependencies]
refract-web = {{ {web} }}
wasm-bindgen = {bindgen}
web-sys = {{ version = {web_sys}, features = ["Element", "console"] }}

[features]
desktop = ["dep:refract-desktop"]

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
refract-desktop = {{ {desktop}, optional = true }}

[[bin]]
name = "{binary}-desktop"
path = "src/desktop.rs"
required-features = ["desktop"]

[profile.dev]
debug = 0
incremental = false
"#,
        name = quote(name),
        rust_version = quote(&rust_version),
        refract = dependency("refract"),
        web = dependency("web"),
        desktop = dependency("desktop"),
        bindgen = version("wasm-bindgen")?,
        web_sys = version("web-sys")?,
        binary = name
    );
    // Claim the destination before writing; never merge with an existing project.
    if let Some(parent) = options.path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&options.path)?;
    let write = || -> Result<()> {
        fs::create_dir(options.path.join("src"))?;
        fs::create_dir(options.path.join("public"))?;
        fs::write(options.path.join("Cargo.toml"), manifest)?;
        fs::write(
            options.path.join("src/lib.rs"),
            include_str!("../templates/lib.rs.txt"),
        )?;
        fs::write(
            options.path.join("src/desktop.rs"),
            include_str!("../templates/desktop.rs.txt")
                .replace("{{crate}}", &crate_name)
                .replace("{{name}}", name),
        )?;
        fs::write(
            options.path.join("public/index.html"),
            include_str!("../templates/index.html")
                .replace("{{crate}}", &crate_name)
                .replace("{{name}}", name),
        )?;
        fs::write(
            options.path.join(".gitignore"),
            "/target/\n/dist/\n/public/pkg/\n/.tools/\n",
        )?;
        let source = match &framework {
            Some(framework) => format!(
                "local framework dependencies from `{}`",
                framework.display()
            ),
            None => format!("Refract {} from crates.io", env!("CARGO_PKG_VERSION")),
        };
        fs::write(options.path.join("README.md"), format!("# {name}\n\nRun `refract dev` for web or `refract dev --platform desktop` for native.\n\nBoth use the view in `src/lib.rs`. The browser entry point owns its mount;\n`src/desktop.rs` launches the same view in a native window.\n\nDevelopment rebuilds reset state. The CLI watches local path dependencies too.\n\nBuild for distribution with `refract build --release` (web output: `dist/web`)\nor `refract build --release --platform desktop` (native output: `dist/desktop`).\nServe the web output with any static HTTP server. For a URL prefix, pass\n`--base-path /my-app/` and use relative asset URLs in `public/index.html`.\nWeb builds copy `public/` and generate `pkg/` without the development reload client.\nDesktop builds copy `assets/` beside the executable; run it from `dist/desktop`\nso relative asset paths resolve. Desktop output targets the current OS and\narchitecture; it is not an installer or signed application bundle.\nBuilds replace previous output for this app and platform after success.\nUse `--out-dir` for a custom path relative to this app directory.\n\nThis project uses {source}.\n"))?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = fs::remove_dir_all(&options.path);
        return Err(error);
    }
    eprintln!(
        "Created {}\n\n  cd {}\n  refract dev\n  refract dev --platform desktop",
        options.path.display(),
        options.path.display()
    );
    Ok(())
}

fn quote(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

fn validate_name(name: &str) -> Result<()> {
    let identifier = name.replace('-', "_");
    let mut chars = identifier.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || [
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
            "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual",
            "yield", "try", "gen",
        ]
        .contains(&identifier.as_str())
        || [
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
    {
        return Err("package names must start with an ASCII letter, contain only letters, digits, '-' or '_', and not be reserved names".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn scaffold_is_valid_and_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hello-app");
        create(NewOptions {
            path: path.clone(),
            name: None,
            framework_path: None,
        })
        .unwrap();
        let original = fs::read_to_string(path.join("Cargo.toml")).unwrap();
        let manifest: toml::Value = toml::from_str(&original).unwrap();
        assert_eq!(manifest["package"]["name"].as_str(), Some("hello-app"));
        assert_eq!(manifest["package"]["publish"].as_bool(), Some(false));
        let browser = &manifest["target"]["cfg(target_arch = \"wasm32\")"]["dependencies"];
        let desktop = &manifest["target"]["cfg(not(target_arch = \"wasm32\"))"]["dependencies"];
        for dependency in [
            &manifest["dependencies"]["refract"],
            &browser["refract-web"],
            &desktop["refract-desktop"],
        ] {
            assert_eq!(
                dependency["version"].as_str(),
                Some(env!("CARGO_PKG_VERSION"))
            );
            assert!(dependency.get("path").is_none());
        }
        let package: toml::Value = toml::from_str(include_str!("../../Cargo.toml")).unwrap();
        let versions = &package["package"]["metadata"]["refract"]["scaffold"];
        assert_eq!(browser["wasm-bindgen"], versions["wasm-bindgen"]);
        assert_eq!(browser["web-sys"]["version"], versions["web-sys"]);
        assert!(fs::read_to_string(path.join("src/desktop.rs"))
            .unwrap()
            .contains("hello_app::application()"));
        assert!(create(NewOptions {
            path: path.clone(),
            name: None,
            framework_path: None
        })
        .is_err());
        assert_eq!(
            fs::read_to_string(path.join("Cargo.toml")).unwrap(),
            original
        );
    }

    #[test]
    fn explicit_checkout_uses_its_paths_and_browser_versions() {
        let temp = tempfile::tempdir().unwrap();
        let framework = temp.path().join("framework with spaces");
        for name in ["refract", "web", "desktop"] {
            let path = framework.join("crates").join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("Cargo.toml"), "").unwrap();
        }
        fs::write(
            framework.join("Cargo.toml"),
            r#"
[workspace.package]
rust-version = "1.93"
[workspace.dependencies]
wasm-bindgen = "=0.2.100"
web-sys = { version = "=0.3.77" }
"#,
        )
        .unwrap();
        let path = temp.path().join("local-app");
        create(NewOptions {
            path: path.clone(),
            name: None,
            framework_path: Some(framework.clone()),
        })
        .unwrap();
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(path.join("Cargo.toml")).unwrap()).unwrap();
        let browser = &manifest["target"]["cfg(target_arch = \"wasm32\")"]["dependencies"];
        assert_eq!(
            manifest["dependencies"]["refract"]["path"].as_str(),
            framework
                .canonicalize()
                .unwrap()
                .join("crates/refract")
                .to_str()
        );
        assert!(manifest["dependencies"]["refract"].get("version").is_none());
        assert_eq!(browser["wasm-bindgen"].as_str(), Some("=0.2.100"));
        assert_eq!(browser["web-sys"]["version"].as_str(), Some("=0.3.77"));
    }

    #[test]
    fn invalid_explicit_checkout_does_not_create_an_application() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("invalid-app");
        assert!(create(NewOptions {
            path: path.clone(),
            name: None,
            framework_path: Some(temp.path().to_owned())
        })
        .is_err());
        assert!(!path.exists());
    }

    #[test]
    fn rejects_invalid_crate_identifiers() {
        for name in [
            "", "1app", "my app", "a/b", "type", "self", "CON", "foo\"bar",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        for name in ["my-app", "hello_world", "app2"] {
            assert!(validate_name(name).is_ok());
        }
    }

    #[test]
    fn paths_round_trip_through_toml() {
        let path = Path::new("/a directory/with \"quotes\"/and\\slashes");
        let value: toml::Value =
            toml::from_str(&format!("path = {}", quote(&path.to_string_lossy()))).unwrap();
        assert_eq!(value["path"].as_str(), path.to_str());
    }
}
