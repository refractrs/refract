# Changelog

Notable changes to Refract, newest first.

## 0.1.7

- Add portable vector drawing with shared, copy-on-write paths, curves,
  arcs, fills, gradients, and configurable strokes. Add `Shape`, built-in
  shapes, and `shape(...)` views that generate geometry from layout bounds.
- Add `canvas` and `canvas_with` with independently tracked input
  preparation. Drawing uses committed input snapshots without evaluating
  views or reading the attribute graph on animation frames.
- Add `.animatable_inputs()` and the `Animatable` trait for custom canvas
  data. Use existing animation transactions and preserve displayed values
  and spring velocity when retargeting. Interpolate colors with
  premultiplied alpha to avoid dark fringes.
- Add scoped transforms, clipping, isolated layers, alpha masks, Gaussian
  blur, opacity, and blend modes. Validate drawing commands before they
  reach a renderer, and clip canvas ink to its content box.
- Render canvases through Vello on desktop and retained SVG nodes on the
  web. Include initial vector drawings in prerendered output and support
  adopting canvas SVG during hydration.
- Cache unchanged drawings and add on-change, animated, and paused canvas
  schedules. Preserve animation phase while paused or hidden. Honor
  reduced-motion preferences for continuous canvas schedules on the web
  and macOS.
- Expose local pointer positions and bounds through `ActionContext`, using
  host-provided geometry for each handler in the bubbling route. Native
  pointer hit testing follows displayed transforms.
- Add a shared desktop and web prism example with spring-driven pointer
  tilt, animated light pulses, gradients, masks, and glow controls. Add
  coverage for canvas invalidation, caching, pointer geometry, prerendering,
  and native GPU effects at multiple display scales.
- Organize core modules into view, runtime, and presentation directories
  alongside drawing, preserving crate-root exports and the prelude.

See [the canvas example](examples/canvas/README.md) for the drawing API,
animation behavior, and backend limitations.

## 0.1.6

- Add portable animations with linear and easing curves, configurable
  springs, and delays. Interrupted animations continue from the displayed
  position; springs also preserve velocity.
- Add `with_animation`, `with_transaction`, value-scoped `.animation(...)`,
  and subtree transaction overrides. Animation intent survives deferred
  updates without leaking between independent view branches.
- Separate presentation from application state. Desktop and web share
  interpolation, animation frames do not reevaluate views or rerun effects,
  and hosts stop requesting frames when motion settles.
- Add `offset`, `scale_effect`, `rotation_effect`, and `transform_anchor`.
  Native drawing, clipping, input, and IME placement follow the displayed
  transforms without changing layout allocation.
- Animate opacity, explicit solid colors, corner radii, numeric dimensions
  and constraints, padding, gap, and compatible borders and shadows. Keep
  displayed geometry valid during spring overshoot.
- Add opacity, offset, and scale transitions, with composition and separate
  insertion and removal effects. Removal releases handlers, focus, effects,
  and scoped work immediately while an inert snapshot finishes the exit.
  Initial rendering and hydration remain immediate.
- Honor subtree transaction overrides during removal, including overrides
  that disable an inherited exit animation.
- Keep web exit snapshots from repainting ancestor backgrounds, borders,
  and shadows over surviving content.
- Add deterministic presentation time to `refract_test::TestHost`, a shared
  desktop and web animation playground, browser animation checks, and
  native GPU coverage for transforms and opacity.
- Add subtle chat selection and composer colors, conversation fades, and
  message entrances. Typing and streamed tokens remain immediate. Clear
  deselected sidebar highlights immediately to avoid a flash when creating
  a new conversation.
- Animate the chat sidebar's width and slide on wide windows, with a short
  slide and fade on compact windows. Preserve the full-height conversation
  list and release hidden sidebar controls immediately.
- Move the active conversation title into the chat header. Remove the
  duplicate title row, local demo badge, and sidebar tagline.

Reduced-motion preference integration is deferred. Automatic layout and
list-reorder transitions, intrinsic-size interpolation, matched geometry,
keyframes, repetition, and custom animatable data are not included.

See [the animation playground](examples/animation/README.md) for the API,
supported properties, and lifecycle behavior.

## 0.1.5

- Distinguish pointer focus from keyboard navigation. Pointer-focused
  editors use a quiet editing outline, pointer-focused buttons omit the
  ring, and Tab restores the stronger navigation indicator.
- Add `FocusRing`, `focus_ring`, and `focus_within` to configure indicators
  and let compound controls draw focus around their outer surface. Rings
  follow corner radii and do not occupy layout space.
- Reveal native keyboard focus through nested scroll viewports using the
  current layout. Recover focus to a surviving tab stop when the focused
  control is removed or disabled on either host.
- Add `focused` and `focused_value` modifiers backed by graph bindings.
  Applications can request and observe focus through ordinary state, with
  direct and subtree focus notifications batched together. Expose window
  activation separately through `Surface`.
- Update chat to focus the composer for new conversations, share focus
  targets, and draw its focus indicator around the composer container.
- Refine the native text editing menu with measured label and shortcut
  columns, platform-specific shortcut hints, a translucent background,
  backdrop blur, rounded corners, and a soft shadow. Remove Select All
  from the menu while retaining its keyboard shortcut.

## 0.1.4

- Support native single-line text truncation with an ellipsis. Choose cut
  points at grapheme boundaries and retain full-text intrinsic sizing.
- Add independent horizontal and vertical alignment for `z_stack`
  children, preserving default stretch behavior and declaration-order
  layering on desktop, web, and prerendered output.
- Skip reconciliation work for clean memoized subtrees whose inputs and
  environment scope are unchanged. Retain mounted state-slot maps across
  updates while continuing to process dirty descendants.
- Avoid repeated ancestor walks during dirty-path tracking and quadratic
  dependency capture for large read sets. Reuse unchanged dependency lists.
- Reduce reconciliation allocations with lightweight view metadata and
  direct matching of unchanged sibling prefixes. Preserve keyed reorders
  and the description fallback for custom views.
- Add native incremental rendering with damage tracking and retained Vello
  output. Repaint affected regions and reuse unchanged frames, with full
  redraws when damage cannot be bounded safely. Add an opt-in
  `debug-damage` Cargo feature to visualize repainted regions.

## 0.1.3

- Add `refract build --prerender` for web builds. Evaluate the initial view
  from the application's WebAssembly module and emit HTML and a snapshot
  alongside the browser runtime.
- Add DOM hydration and wire it into generated projects. Reuse matching
  nodes, fall back to a fresh mount on mismatches, and start effects only
  after the browser commits the view.
- Add circular `RadialGradient` backgrounds through the shared `Gradient`
  type, with configurable centers, radii, and color stops. Support CSS,
  prerendered HTML, and native rendering while preserving existing linear
  gradient modifier calls.
- Add `italic`, `letter_spacing`, and `word_spacing` to text and fields,
  including inherited typography, editable placeholders, and native
  synthetic italic rendering when a font has no italic face.
- Add shared `blur` and `backdrop_blur` modifiers for desktop and web,
  including prerendered styles.
- Prevent text selection on web activation targets so repeated clicks do
  not select button labels.

## 0.1.2

- **Breaking:** rename `refract-desktop` to `refract-native` and `Desktop`
  types to `Native`. Update dependencies and type names when upgrading.
  The CLI continues to use `--platform desktop` for the native target.
- Add a reactive `Surface` in the root environment for viewport, title,
  and application URL observations without depending on a host adapter.
- Add title and push/replace navigation requests applied after commit.
  Browser mounts opt into document control; native hosts maintain
  application history and can accept incoming URLs.
- Share the read implementation between `Read` and `Binding`, preserving
  binding methods and deref coercion.
- Update chat with a responsive sidebar and URL-driven conversation
  selection. Use blue focus and selection defaults across both adapters.

## 0.1.1

- Add `refract build` for web assets and native executables, with release
  profile selection and an optional web base path. Stage output before
  replacing the previous build, reject unsafe output locations, and keep
  the last successful output when compilation or packaging fails.
- Add typed channels for delivering background results to callbacks on the
  UI thread. Process messages in bounded batches and wake the host when
  work arrives. Dropping a subscription or disposing the app closes
  delivery and discards queued results.
- Replace chat UI timer callbacks with native workers and browser async
  tasks. Give each conversation ownership of its stream and cleanup,
  reject queued results after Stop, and retry a full inbox.
- Keep native image requests pending when the worker queue is full instead
  of permanently failing the mounted resource. Admit deferred requests as
  capacity becomes available and skip requests that have unmounted.
- Add CI coverage for native targets and WebAssembly builds.

## 0.1.0

- Initial release of the portable Rust UI framework, with a reactive
  attribute graph, typed view composition, graph-backed state and bindings,
  environment providers, mounted effects, keyed identity, and memoization.
- Provide browser DOM and desktop adapters, plus an in-memory test host.
  Share views across platforms while keeping layout and rendering in the
  adapters.
- Include text, editable fields, images, stacks, overlays, scrolling, input
  handling, and composable layout, paint, and typography modifiers.
- Provide view authoring macros alongside handwritten `View`
  implementations.
- Add `refract new` for project scaffolding and `refract dev` for watched
  browser and native development.
- Include counter, clock, gallery, and chat examples.
