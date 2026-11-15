# Refract UI

`refract-ui` is reserved for Refract's default component collection. The crate
currently exports no components, and its `refract-ui.toml` catalog is empty.

The `refract ui` CLI can still install editable source from other collections.

## Installing editable components

These examples assume your collection provides a `button` component:

```sh
refract ui list --from ../my-components
refract ui add button --from ../my-components
refract ui add button --from ../my-components --dry-run
refract ui add button --from ../my-components -p my-app
```

Bare names resolve the published `refract-ui` version matching the application's
resolved `refract` version unless a default source is configured. `--version`
selects another UI version requirement, subject to the catalog's compatibility
check. Use `--from` or a registered source while the default collection is empty.

The installer creates `src/ui/`, declares the module in the library crate root
(or the sole binary's root), and copies the selected component dependencies.
Use the resulting source like any other application module:

```rust,ignore
use crate::ui::button::button;
```

The application depends on `refract` and any additional crates declared by the
selected components. Copying source does not add a dependency on `refract-ui`.
Cargo resolves source downloads in a temporary project without modifying the
application's `Cargo.lock` or compiling downloaded code. Run your usual Cargo
check or build afterward to resolve any newly added application dependencies.

`--dry-run` performs resolution and prints the file plan without changing project
files. `--offline` permits cached Cargo packages and local collections, and
rejects Git fetches. Source downloads can populate Cargo's normal cache.
`--platform web` or `--platform desktop` checks every selected component and its
transitive component dependencies. Omitting this option accepts either platform;
`ui list` shows each item's declared support.

Commit the installed source and `refract-ui.lock`. The lock records source
identities, resolved Git commits or crate versions, component dependencies,
licenses, and SHA-256 hashes of the original copied files. It tracks provenance;
it does not replace `Cargo.lock` or fetch components during builds. Repeating
`add` preserves local changes when the upstream bytes are unchanged. Changed
upstream files conflict with differing local files; inspect the changes and use
`--overwrite` to replace them explicitly. Files owned by another component or
source remain conflicts. Installation does not delete files or merge source.

## Other sources

```sh
refract ui add button --from ../my-components
refract ui add github:acme/refract-ui/button --rev v0.2.0
refract ui add github:acme/refract-ui
refract ui add --from git:https://example.com/team/ui.git --all

refract ui source add acme github:acme/refract-ui --rev v0.2.0
refract ui list @acme
refract ui add @acme/button
```

A repository without a component opens a text picker on an interactive terminal.
Scripts must supply component names or `--all`. Each invocation uses one source;
component dependencies resolve within that source at the same revision. Git
sources require Git and use its existing authentication configuration. No
checkout, filters, install scripts, or submodules are run.

Sources are saved in the selected package's Cargo metadata. Registering a source
only records its address; `list` and `add` validate its catalog. Local paths passed
to `--from` resolve from the working directory. Registered relative paths resolve
from the package directory; `source add` stores local paths as absolute paths.
For a portable shared repository configuration, write a relative path manually.

```toml
[package.metadata.refract.ui]
directory = "src/components"
entry = "src/lib.rs"
default-source = "acme"

[package.metadata.refract.ui.sources]
acme = { source = "github:acme/refract-ui", rev = "v0.2.0" }
local = "../my-components"
```

The directory must sit beside the chosen crate root. Existing ordinary external
module declarations are preserved. Inline modules, conditional declarations,
redirected `#[path]` modules, and a sibling `components.rs` require choosing a
separate directory. The installer does not rewrite arbitrary Rust imports.

## Publishing a collection

Place a `refract-ui.toml` catalog at the root of a local directory or Git
repository. For a crate source distribution, include it in the published package
along with every referenced source file and license. For example:

```toml
schema = 1
name = "acme"
license = "MIT"
license-file = "LICENSE"

[compatibility]
refract = ">=0.1.8, <0.2.0"

[items.theme]
description = "Shared design tokens"
platforms = ["web", "desktop"]
files = [{ source = "src/theme.rs", target = "theme.rs" }]

[items.button]
description = "Primary button"
platforms = ["web", "desktop"]
requires = ["theme"]
files = [{ source = "src/button.rs", target = "button.rs" }]

# Optional ordinary Cargo dependencies, with additive features.
[items.button.dependencies.serde]
version = "1"
features = ["derive"]
default-features = true
```

Targets are relative to the application's configured UI directory. Each Rust
module must have exactly one entry, such as `button.rs` or `button/mod.rs`.
Multi-file modules must include their own nested module declarations. Additional
assets can be copied beside the source. The installer maintains the outer
`mod.rs` and reserves `licenses/` for copied license texts. Absolute paths,
parent traversal, symlinks, cycles, and duplicate destinations are rejected.

Use sibling imports such as `super::theme` so source compiles unchanged inside
both the collection crate and the application. Item names must be Rust module
identifiers. Catalog schema 1 accepts ordinary crates.io dependencies with
versions, features, and default-feature settings. Existing workspace inheritance
and dependency sources are preserved; differing version requirements, renamed
or optional dependencies, and required-but-disabled default features produce an
error for manual reconciliation. Target-specific dependency declarations are not
part of schema 1. The application's direct `refract` dependency currently must
be unconditional, unrenamed, and sourced from crates.io or a local path.

Refract UI shares the workspace release version. Its package retains the empty
catalog, library entry point, README, and license for future components. The CLI
resolves this collection at runtime and does not depend on the crate.
