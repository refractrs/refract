# refract-parley

This is Refract's temporary fork of
[Parley](https://github.com/linebender/parley), based on the crates.io release
of `parley` 0.12.0. It is published as `refract-parley` and maintained in
[`vendor/parley`](https://github.com/refractrs/refract/tree/main/vendor/parley).
Report issues with this fork to the
[Refract repository](https://github.com/refractrs/refract/issues).

The only source change is `PlainEditor::set_ranged_styles` and its backing
storage in `src/editing/editor.rs`. This lets the host supply externally owned
formatting while retaining Parley's caret layout, selection, hit testing, and
IME editing. Spans use UTF-8 byte offsets and must be refreshed by the host
after edits. Refract intends to return to upstream Parley once a published
release offers equivalent externally supplied editor style ranges.

The fork's version is independent of Refract's release version. Its standalone
manifest keeps it outside the Refract workspace. Publish it before dependent
Refract crates:

```sh
cargo publish --manifest-path vendor/parley/Cargo.toml
```

Dependents can retain the `parley` import name:

```toml
parley = { package = "refract-parley", version = "0.12.0" }
```

Upstream copyright notices and the Apache-2.0 and MIT licenses are retained.
The upstream README follows.

---

<div align="center">

# Parley

**Rich text layout**

[![Latest published parley version.](https://img.shields.io/crates/v/parley.svg)](https://crates.io/crates/parley)
[![Documentation build status.](https://img.shields.io/docsrs/parley.svg)](https://docs.rs/parley)
[![Dependency staleness status.](https://deps.rs/crate/parley/latest/status.svg)](https://deps.rs/crate/parley)
[![Linebender Zulip chat.](https://img.shields.io/badge/Linebender-%23parley-blue?logo=Zulip)](https://xi.zulipchat.com/#narrow/channel/205635-parley)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)

</div>

Parley provides an API for implementing rich text layout.
It is backed by [HarfRust](https://github.com/harfbuzz/harfrust) for text shaping.

## Minimum supported Rust Version (MSRV)

This version of Parley has been verified to compile with **Rust 1.88** and later.

Future versions of Parley might increase the Rust version requirement.
It will not be treated as a breaking change and as such can even happen with small patch releases.

<details>
<summary>Click here if compiling fails.</summary>

As time has passed, some of Parley's dependencies could have released versions with a higher Rust requirement.
If you encounter a compilation issue due to a dependency and don't want to upgrade your Rust toolchain, then you could downgrade the dependency.

```sh
# Use the problematic dependency's name and version
cargo update -p package_name --precise 0.1.1
```
</details>

## Community

Discussion of Parley development happens in the [Linebender Zulip](https://xi.zulipchat.com/), specifically the [#parley channel](https://xi.zulipchat.com/#narrow/channel/205635-parley).
All public content can be read without logging in.

Contributions are welcome by pull request. The [Rust code of conduct] applies.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache 2.0 license, shall be licensed as noted in the [License](#license) section, without any additional terms or conditions.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
