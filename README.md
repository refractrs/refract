# Refract

[Website](https://refract.rs) · [Documentation](https://refract.rs/docs)

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

## Build for distribution

```sh
refract build --release
refract build --release --platform desktop
```

Serve `dist/web/` with a static HTTP server. Desktop builds put the executable
and `assets/` in `dist/desktop/`; run the executable from that directory.

Use `--base-path /my-app/` when hosting under a URL prefix, or `--out-dir` to
choose an output directory. Run `refract build --help` for all options.

## License

[MIT](https://github.com/refractrs/refract/blob/HEAD/LICENSE).
