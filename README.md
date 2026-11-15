# Refract

[Website](https://refract.rs) · [Documentation](https://refract.rs/docs)

Refract provides browser rendering through `refract-web` and native rendering
through `refract-native`. The native adapter currently targets desktop platforms.

## Get started

### From crates.io

Install the CLI from the `refract` package and create an app:

```sh
cargo install refract --features cli --locked
refract new my-app
cd my-app
refract dev --open
```

Use `refract dev --platform desktop` to run the app natively. To add the library
to an existing project, run `cargo add refract`.

### From a checkout

```sh
git clone https://github.com/refractrs/refract.git
cd refract
cargo install --path crates/refract --features cli --locked
refract dev -p refract-counter --open
```

To run the same app on desktop:

```sh
refract dev -p refract-counter --platform desktop
```

The CLI watches for changes and rebuilds your app. To create a new project against
this checkout, run `refract new ../my-app --framework-path .`, then `cd ../my-app`
and `refract dev --open`.

The web development server serves `public/index.html` when opening or refreshing
an extensionless client-side route. Missing assets and development endpoints
still return 404. Route fallback pages use `/` as the asset base unless the page
defines its own `<base>` element, so relative scripts and styles load correctly.

### Build for distribution

```sh
refract build --release
refract build --release --platform desktop
```

Serve `dist/web/` with a static HTTP server. Desktop builds put the executable
and `assets/` in `dist/desktop/`; run the executable from that directory.

Use `--base-path /my-app/` when hosting under a URL prefix, or `--out-dir` to
choose an output directory. Run `refract build --help` for all options.

## UI components

Copy components from a local or Git collection into your app and customize
their source with `refract ui`:

```sh
refract ui list --from ../my-components
refract ui add button --from ../my-components
```

`refract-ui` is reserved for the default collection and currently contains no
components. Use `--from` or a registered source until components are available.

Components are installed into `src/ui/` with shared dependencies, module
declarations, licenses, and a `refract-ui.lock` provenance file. Existing local
edits are preserved. Use `--dry-run` to inspect planned changes, `-p` to select
a workspace package, and `--from` to install a local or Git collection.

See [Refract UI](crates/ui/README.md) for usage and the source catalog format.

## Examples

The [canvas example](examples/canvas/README.md) demonstrates portable vector
drawing, reactive inputs, and presentation animation on web and desktop.

The [shader example](examples/shader/README.md) renders application-owned WGSL
with reactive uniforms, animation, and image sampling on native GPUs and WebGPU.

## License

[MIT](https://github.com/refractrs/refract/blob/HEAD/LICENSE).
