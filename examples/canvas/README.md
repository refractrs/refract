# Canvas

Run the same vector drawing in a browser or on desktop:

```sh
cargo run -p refract --features cli -- dev -p refract-canvas
cargo run -p refract --features cli -- dev -p refract-canvas --platform desktop
```

To launch the native binary directly:

```sh
cargo run -p refract-canvas --features desktop --bin refract-canvas-desktop
```

Move the pointer over the prism to tilt its projected vertices. Pause the light
pulses, toggle the glow, or reset the tilt. Both hosts use the same Rust drawing
code. The example exercises gradient fills and strokes, alpha masks, Gaussian
blur, dashes, view-box fitting, and animated input parameters.

## Shapes and static drawings

```rust
use refract::prelude::*;

let dot = shape(Circle)
    .fill(Color::BLUE)
    .stroke(Color::WHITE, StrokeStyle::new(2.0))
    .width(40.0)
    .height(40.0);

let illustration = canvas(|g, frame| {
    g.fill(&Path::ellipse(frame.bounds), Color::BLUE);
})
.view_box(Rect::new(0.0, 0.0, 200.0, 120.0))
.width(400.0)
.height(240.0);
```

A `Shape` implements `path(bounds) -> Path`. Built-in shapes are `Circle`,
`Ellipse`, and `Rectangle`; `Path` itself also implements `Shape`. Shapes default
to a black fill. Use `.no_fill()` for an outline. A shape uses the same canvas
backend, with normal view layout, events, and accessibility metadata.

Give canvases a size through explicit dimensions or their parent's layout. They
have no intrinsic size. Geometry uses logical pixels with a top-left origin and
y increasing downward. A view box fits and centers its coordinates while
preserving aspect ratio. `CanvasFrame.size` is the allocated content size and
`bounds` is the logical view box. Padding and borders surround that content.
Canvas ink, including effects, is clipped to the content box on both hosts.

## Reactive inputs and the attribute graph

```rust
use refract::prelude::*;

#[view]
fn illustration(cx: &mut ViewContext<'_>, center: Binding<Point>) -> impl View {
    canvas_with(
        move |inputs| inputs.read(&center),
        |g, _frame, center| {
            let circle = Path::ellipse(Rect::new(
                center.x - 10.0, center.y - 10.0, 20.0, 20.0,
            ));
            g.fill(&circle, Color::BLUE);
        },
    )
    .animatable_inputs()
    .width(200.0)
    .height(120.0)
}
```

The first closure runs under the canvas mount's existing observer. Its reads
subscribe that mount, not the parent body. Dynamic reads replace dependencies
only when the candidate update commits. Inputs must be `Clone + PartialEq`;
large immutable assets can be shared through `Rc` or `Arc`.

`DrawingInputs` exposes `read`, `with`, and `environment`. The drawing closure
receives a committed input snapshot, frame information, and a graphics context.
It cannot read or mutate the graph through that context. Keep both closures
pure: drawing can run again after resize, and preparation can be discarded.

Drawing commands are cached until inputs, size, scheduled time, or the drawing
callback changes. Input changes do not require layout when the box style is
unchanged. Animation frames do not evaluate view bodies or advance the semantic
revision. The attribute graph crate is unchanged.

## Animation and scheduling

`.animatable_inputs()` opts into existing `with_animation` and `.animation(...)`
transactions. State holds the target; presentation samples the drawing inputs.
Initial values and updates without animation appear immediately. Retargeting a
spring preserves its displayed value and velocity. Animation cancellation and
unmount release retained tracks.

`Animatable` is implemented for `f32`, `f64`, `Point`, `Size`, `Color`, and pairs
of animatable values. Custom implementations return ordered scalar components
and rebuild a value from them. Component counts must remain stable and values
must be finite. Other fields in a custom value snap to the new target. This
animates geometry parameters; arbitrary path morphing is not provided.

Schedules are independent of input animations:

- `CanvasSchedule::OnChange` is the default: no clock and no idle frames.
- `CanvasSchedule::animation()` requests frames and supplies visible elapsed time.
- `CanvasSchedule::Paused` freezes that elapsed time without requesting frames.

Hidden canvases pause their clock and resume without a jump. Browser canvases
observe intersection, document visibility, resizing, and reduced-motion media
queries. The macOS host samples the accessibility motion preference when drawing;
other native platforms do not yet bridge that OS preference. Reduced motion
suppresses the continuous canvas schedule, not explicit input animations.

## Drawing operations

`Path` supports moves, lines, quadratic and cubic Béziers, closing contours,
polylines, polygons, rectangles, ellipses, and elliptical arcs. Arc angles are in
radians and positive sweeps run clockwise. Paths share storage until modified.

`Paint` supports solid colors and coordinate-based linear/radial gradients.
`StrokeStyle` controls width, caps, joins, miter limit, dashes, and dash offset.
Dash lengths and offsets use drawing units. Use `FillRule::EvenOdd` for holes.

`GraphicsContext` records portable commands:

- `fill`, `fill_rule`, and `stroke` draw paths.
- `transformed`, `clipped`, and `scoped` delimit drawing state.
- `layer` isolates children, applies blur, applies an alpha mask, and composites
  with group opacity and the chosen blend mode, in that order.

Masks use paint alpha; RGB does not change coverage. Blur is Gaussian standard
deviation in local units, applied before the group's transform. The native
renderer bounds effect surfaces to their drawing coverage and caps blur texture
resolution. Invalid geometry, gradients, stroke styles, and effects return a
rendering error instead of reaching the rasterizer unchecked.

A canvas is one event/accessibility target. Individual paths do not become
controls. Use separate shape views for separate controls. During pointer events,
`ActionContext::local_position(event.position)` and `local_bounds()` describe the
current handler's border box, including transforms and scrolling. Apply padding
and the inverse view-box transform if you need drawing-space coordinates.

## Validation

Core and test-host tests cover dependency isolation, discarded preparations,
dynamic subscriptions, cleanup, caches, scheduling, and input animation. Native
host tests verify that drawing animation does not rerun layout or graph work.
GPU pixel tests cover alpha masks, blur, and dashed strokes at multiple scales:

```sh
cargo test -p refract-native gpu_canvas -- --ignored
```

Web tests cover SVG resources and prerendered output. Browser and desktop use
platform rasterizers, so small antialiasing differences are expected.
