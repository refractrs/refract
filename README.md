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

### Build for distribution

```sh
refract build --release
refract build --release --platform desktop
```

Serve `dist/web/` with a static HTTP server. Desktop builds put the executable
and `assets/` in `dist/desktop/`; run the executable from that directory.

Use `--base-path /my-app/` when hosting under a URL prefix, or `--out-dir` to
choose an output directory. Run `refract build --help` for all options.

## Examples

The [canvas example](examples/canvas/README.md) demonstrates portable vector
drawing, reactive inputs, and presentation animation on web and desktop.

## Host inputs

An embedding host can own reactive state while views receive read-only fields
through an environment object. Initialize that state in the application's own
runtime before the first render:

```rust
use refract::prelude::*;
use std::rc::Rc;

struct Inputs {
    dark: Read<bool>,
}

fn initialize(rt: &mut Runtime) -> (impl View, State<bool>) {
    let dark = State::new(rt, false);
    let app = view_fn(|cx| {
        let inputs = cx.object::<Inputs>()?;
        Ok::<_, ViewError>(text(if cx.read(&inputs.dark) { "Dark" } else { "Light" }))
    })
    .environment_object(Rc::new(Inputs { dark: dark.read_only() }));
    (app, dark)
}
```

On the web, call `refract_web::mount_with_inputs(host, options, initialize)`.
The returned `InputMount` owns both the mount and the host handles. Publish
changes with `mounted.update_inputs(|dark, rt| dark.set_if_changed(rt, true))`.
Each call uses the existing queued mutation batch. Unchanged values do not
invalidate views, local view state survives updates, and disposed mounts reject
new work. Use `mounted.mount()` for the ordinary mount API.

For prerendering, export the same initializer with
`refract_web::export_prerender_with_inputs!(initialize)` and start the browser
with `hydrate_with_inputs(host, options, initialize)`. Both must use identical
initial values; publish the host's newer values after hydration. Initialization
must not access browser APIs, and effects only start in the live mount. The
snapshot remains HTML-only: inputs are recreated, not serialized automatically.
A theme change after hydration can still change the first visible colors; a
matching initial palette requires a themed snapshot or theme-aware CSS.

Custom hosts and tests can use `App::with_inputs(initialize)` to receive the
application and handles separately, then update them through `App::mutate`.
This API reuses `State`, `Read`, and environment objects without adding a second
state system. JavaScript serialization and app-to-host requests belong to the
application's host adapter.

## License

[MIT](https://github.com/refractrs/refract/blob/HEAD/LICENSE).
