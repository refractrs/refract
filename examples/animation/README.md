# Animation

Run the same playground in a browser or on desktop:

```sh
cargo run -p refract --features cli -- dev -p refract-animation
cargo run -p refract --features cli -- dev -p refract-animation --platform desktop
```

Reverse motion during a spring, expand the panel, and toggle the notice. State
changes immediately; only presentation interpolates. Frames do not evaluate views,
rerun effects, or change the application's semantic revision.

## Authoring

```rust
use refract::prelude::*;
use std::time::Duration;

// Inside an action; `expanded` is a Binding<bool>.
with_animation(cx.runtime(), Animation::spring(), |rt| {
    expanded.update(rt, |value| *value = !*value);
});

// Override animation for this subtree when `selected` changes after mounting.
text("Selected")
    .opacity(if selected { 1.0 } else { 0.5 })
    .scale_effect(if selected { 1.05 } else { 1.0 })
    .animation(Animation::ease_out(Duration::from_millis(180)), selected);
```

`Animation` provides `linear`, `ease_in`, `ease_out`, `ease_in_out`, `spring`,
`spring_with(response, damping_ratio)`, and `delay`. Timed curves retarget from the
current displayed value; springs also preserve velocity. Durations use
`std::time::Duration`. A zero timed duration snaps after its delay.

`with_transaction` scopes synchronous writes. An asynchronous completion needs its
own transaction. Context survives deferred batches and discarded preparations.
Independent view branches retain their own context. When several writes affect
one view evaluation, the latest relevant mutation wins. `.animation(..., value)`
is a subtree scope, and all animatable changes in that update participate. It is
not a promise to preserve each chained style modifier as a separate animation
node. Place style modifiers before the animation boundary when they should belong
to that boundary. Initial rendering and hydration are immediate.

Use `.transaction(|tx| tx.disables_animations = true)` to suppress animation in a
subtree. Setting `tx.animation = None` clears inherited animation but allows a
nested value-scoped modifier to enable its own animation. Transaction closures
must be pure. Animation scopes do not introduce DOM or native layout boxes.

## Supported properties

Opacity; offset, scale and rotation; corner radii; numeric width/height and size
constraints; padding/gap; explicit solid foreground/background colors; compatible
borders and box shadows. Colors interpolate in premultiplied sRGB. Opacity and
nonnegative geometry are clamped at presentation, allowing spring overshoot in
translation, scale and rotation. Compatible optional values interpolate; changes
to intrinsic sizing or inherited colors snap. Gradients, text content, font
metrics, and images are not interpolated.

Transforms use logical pixels and degrees and do not change layout allocation.
They compose as translation × rotation × scale around `UnitPoint::CENTER`, which
can be changed with `.transform_anchor(...)`. Nested views compose outside-in.
Native drawing, clipping, hit testing, and IME placement use presentation geometry.
Transform-containing native frames conservatively repaint fully; ordinary
paint-only animations do not run layout or shape text.

## Transitions and lifetime

`.transition(Transition::opacity())` describes an insertion/removal effect under
an animated transaction. Combine opacity, offset and scale with `combined_with`,
or use `Transition::asymmetric(insertion, removal)`. Initial mount does not animate.
Insertion starts from the effect and approaches the normal view. Removal cleans
up effects, focus, handlers and scoped asynchronous work immediately. An inert
visual snapshot finishes the exit above the surviving content, outside layout.
It retains image/glyph resources only until completion. Reinsertion after removal
creates a fresh logical mount even with the same key; it does not revive disposed
state. Ancestor transitions subsume exiting descendants.

Both adapters stop requesting animation frames when settled. Hidden browser tabs
and occluded native windows do not replay missed frames; elapsed time catches up
when presentation resumes. Disposal cancels all pending presentation work.

Automatic list/layout transitions, intrinsic-size interpolation, matched geometry,
keyframes, repetition, custom animatable data and reduced-motion preferences are
outside this release's scope.

## Testing

`refract_test::TestHost::advance_time` advances deterministic presentation time.
`presented(id)` reads a displayed element; `elements` continues to hold logical
targets. Browser smoke tests exercise actual frames, interruption, idle scheduling,
transitions, and disposal in Chromium.
