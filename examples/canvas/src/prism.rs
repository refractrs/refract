//! A portable vector illustration: no SVG strings or platform drawing calls.

use refract::prelude::*;

fn project(x: f64, y: f64, z: f64, tilt: Point) -> Point {
    let camera = 650.0;
    let yaw = tilt.x * 0.34;
    let pitch = -tilt.y * 0.24;

    // Undo the resting projection before rotation, preserving the original silhouette.
    let x = (x - 265.0) * (1.0 - z / camera);
    let y = (y - 166.0) * (1.0 - z / camera);
    let rx = x * yaw.cos() + z * yaw.sin();
    let rz = z * yaw.cos() - x * yaw.sin();
    let ry = y * pitch.cos() - rz * pitch.sin();
    let depth = y * pitch.sin() + rz * pitch.cos();
    let perspective = camera / (camera - depth);
    Point::new(
        265.0 + rx * perspective + tilt.x * 5.0,
        166.0 + ry * perspective + tilt.y * 4.0,
    )
}

fn gradient(start: Point, end: Point, stops: &[(f32, Color)]) -> Paint {
    Paint::linear(
        start,
        end,
        stops
            .iter()
            .map(|&(offset, color)| GradientStop::new(offset, color))
            .collect::<Vec<_>>(),
    )
}

fn line(a: Point, b: Point) -> Path {
    Path::polyline([a, b])
}

fn alpha(c: Color, a: f64) -> Color {
    Color(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn pulse(
    g: &mut GraphicsContext,
    points: &[Point],
    color: Color,
    phase: f64,
    start: f64,
    end: f64,
) {
    if !(start..end).contains(&phase) {
        return;
    }
    let t = (phase - start) / (end - start);
    let length = points
        .windows(2)
        .map(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y))
        .sum::<f64>();
    let opacity = (t * 8.0).min(1.0) * ((1.0 - t) * 8.0).min(1.0);

    // One short dash travels along the ray; the long gap keeps it from repeating.
    g.stroke(
        &Path::polyline(points.iter().copied()),
        alpha(color, opacity * 0.8),
        StrokeStyle::new(2.5)
            .line_cap(LineCap::Round)
            .dash(vec![length * 0.12, length])
            .dash_offset(length * (0.12 - t * 1.12)),
    );
}

pub fn draw(g: &mut GraphicsContext, frame: &CanvasFrame, tilt: Point, glow: f64) {
    let top = project(246.0, 54.0, -25.0, tilt);
    let left = project(166.0, 245.0, -25.0, tilt);
    let right = project(359.0, 258.0, -25.0, tilt);
    let facet = project(281.0, 180.0, 105.0, tilt);
    let entry = project(205.0, 160.0, -25.0, tilt);
    let t = (163.0 - 54.0) / (180.0 - 54.0);
    let center = Point::new(top.x + (facet.x - top.x) * t, top.y + (facet.y - top.y) * t);
    let source = Point::new(12.0, 160.0);
    let spread = 1.0 + tilt.x * 0.16 - tilt.y * 0.1;
    let end = |y: f64| Point::new(524.0, center.y + (y - 163.0) * spread);
    let phase = (frame.elapsed.as_secs_f64() / 7.0).fract();

    let spectrum = gradient(
        Point::new(320.0, 105.0),
        Point::new(450.0, 240.0),
        &[
            (0.0, Color::rgb(73, 214, 233)),
            (0.32, Color::rgb(111, 157, 246)),
            (0.65, Color::rgb(151, 129, 237)),
            (1.0, Color::rgb(198, 152, 228)),
        ],
    );
    let fade = Mask::new(
        Path::rectangle(frame.bounds),
        gradient(
            Point::new(390.0, 0.0),
            Point::new(540.0, 0.0),
            &[(0.0, Color::WHITE), (1.0, Color(255, 255, 255, 0))],
        ),
    );

    let grid = StrokeStyle::new(1.0).dash(vec![2.0, 6.0]);
    g.stroke(
        &line(Point::new(12.0, 166.0), Point::new(518.0, 166.0)),
        Color::rgb(215, 225, 238),
        &grid,
    );
    g.stroke(
        &line(Point::new(265.0, 30.0), Point::new(265.0, 300.0)),
        Color::rgb(215, 225, 238),
        &grid,
    );

    g.layer(Layer::new().blur(8.0).opacity(0.17), |g| {
        let rx = 111.0 - tilt.x.abs() * 12.0;
        let ry = 7.0 + tilt.y * 2.0;
        g.fill(
            &Path::ellipse(Rect::new(
                265.0 - tilt.x * 12.0 - rx,
                278.0 - tilt.y * 3.0 - ry,
                rx * 2.0,
                ry * 2.0,
            )),
            Color::rgb(128, 155, 194),
        );
    });
    if glow > 0.001 {
        let beat = 0.18 + 0.06 * (phase * std::f64::consts::TAU).sin();
        g.layer(
            Layer::new()
                .blur(12.0)
                .opacity((beat * glow) as f32)
                .mask(fade.clone()),
            |g| {
                g.fill(&Path::polygon([center, end(94.0), end(257.0)]), &spectrum);
            },
        );
    }
    g.layer(Layer::new().opacity(0.17).mask(fade), |g| {
        g.fill(&Path::polygon([center, end(107.0), end(238.0)]), &spectrum);
    });

    let colors = [
        Color::rgb(73, 185, 207),
        Color::rgb(93, 159, 231),
        Color::rgb(106, 134, 228),
        Color::rgb(143, 123, 216),
        Color::rgb(176, 138, 203),
    ];
    for (i, (color, y)) in colors
        .into_iter()
        .zip([107.0, 140.0, 174.0, 206.0, 238.0])
        .enumerate()
    {
        let start = Point::new(center.x + i as f64 * 2.5, center.y - 5.0 + i as f64 * 3.0);
        g.stroke(&line(start, end(y)), color, StrokeStyle::new(1.5));
        pulse(g, &[start, end(y)], color, phase, 0.30, 0.76);
    }

    let white_light = gradient(
        source,
        Point::new(205.0, 160.0),
        &[
            (0.0, Color(213, 227, 244, 90)),
            (1.0, Color::rgb(183, 207, 233)),
        ],
    );
    g.layer(Layer::new().opacity(0.55), |g| {
        g.stroke(&line(source, entry), &white_light, StrokeStyle::new(9.0))
    });
    g.stroke(&line(source, entry), &white_light, StrokeStyle::new(2.0));

    let glass = gradient(
        Point::new(174.0 + tilt.x * 65.0, 85.0 + tilt.y * 45.0),
        Point::new(336.0 - tilt.x * 35.0, 265.0 - tilt.y * 30.0),
        &[
            (0.0, Color(255, 255, 255, 242)),
            (0.45, Color(226, 237, 249, 166)),
            (0.7, Color(197, 213, 245, 102)),
            (1.0, Color(255, 255, 255, 230)),
        ],
    );
    let body = Path::polygon([top, right, left]);
    g.fill(&body, glass);
    g.stroke(&body, Color::rgb(174, 190, 214), StrokeStyle::new(1.2));

    let side = Path::polygon([top, facet, right]);
    g.fill(
        &side,
        gradient(
            Point::new(245.0, 60.0),
            Point::new(330.0, 270.0),
            &[
                (0.0, Color(181, 199, 225, 30)),
                (1.0, alpha(Color::rgb(179, 201, 244), 0.48 + tilt.x * 0.12)),
            ],
        ),
    );
    g.stroke(&side, Color::rgb(181, 199, 225), StrokeStyle::new(0.7));

    let bottom = Path::polygon([left, facet, right]);
    g.fill(
        &bottom,
        alpha(Color::rgb(230, 239, 255), 0.5 + tilt.y * 0.15),
    );
    g.stroke(&bottom, Color::rgb(181, 199, 225), StrokeStyle::new(0.7));
    g.stroke(&line(entry, center), Color::WHITE, StrokeStyle::new(4.0));
    g.stroke(
        &line(entry, center),
        Color::rgb(140, 174, 217),
        StrokeStyle::new(0.8),
    );
    pulse(g, &[source, entry, center], Color::WHITE, phase, 0.05, 0.34);
    g.stroke(&line(top, left), Color::WHITE, StrokeStyle::new(2.0));
    g.stroke(
        &Path::polyline([top, facet, right]),
        alpha(Color::WHITE, 0.35 + tilt.x * 0.2),
        StrokeStyle::new(1.0),
    );
    g.fill(
        &Path::ellipse(Rect::new(center.x - 4.0, center.y - 4.0, 8.0, 8.0)),
        Color::WHITE,
    );
    if glow > 0.001 {
        g.layer(Layer::new().blur(12.0).opacity((0.65 * glow) as f32), |g| {
            g.fill(
                &Path::ellipse(Rect::new(center.x - 14.0, center.y - 14.0, 28.0, 28.0)),
                Color::WHITE,
            )
        });
    }

    for (x, y) in [(34.0, 40.0), (508.0, 40.0), (34.0, 290.0), (508.0, 290.0)] {
        g.stroke(
            &line(Point::new(x - 10.0, y), Point::new(x + 10.0, y)),
            Color::rgb(169, 186, 210),
            StrokeStyle::new(0.7),
        );
        g.stroke(
            &line(Point::new(x, y - 10.0), Point::new(x, y + 10.0)),
            Color::rgb(169, 186, 210),
            StrokeStyle::new(0.7),
        );
    }
}
