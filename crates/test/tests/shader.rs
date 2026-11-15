use refract_core::{prelude::*, App, CanvasCache, DrawingCommand, ElementKind};
use refract_test::TestHost;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn program() -> Shader {
    Shader::wgsl("struct Uniforms { amount: f32 }\nfn shade(frame: ShaderFrame) -> vec4<f32> { return vec4<f32>(uniforms.amount, frame.uv, 1.0); }").unwrap()
}
fn sample(commands: &[DrawingCommand]) -> (&Shader, Duration) {
    match &commands[0] {
        DrawingCommand::Group { commands, .. } => sample(commands),
        DrawingCommand::Shader {
            shader, elapsed, ..
        } => (shader, *elapsed),
        _ => panic!("expected shader"),
    }
}
#[test]
fn shader_inputs_animate_without_parent_evaluation_and_clock_pauses() {
    let saved = Rc::new(RefCell::new(None));
    let parents = Rc::new(Cell::new(0));
    let preparations = Rc::new(Cell::new(0));
    let shader = program();
    let mut app = App::new(view_fn({
        let (saved, parents, preparations) = (saved.clone(), parents.clone(), preparations.clone());
        move |cx| {
            parents.set(parents.get() + 1);
            let amount = cx.state("amount", || 0.0_f32);
            *saved.borrow_mut() = Some(amount.clone());
            let (shader, preparations) = (shader.clone(), preparations.clone());
            Ok(shader_with(move |inputs| {
                preparations.set(preparations.get() + 1);
                shader.clone().uniform("amount", inputs.read(&amount))
            })
            .animatable_inputs()
            .schedule(ShaderSchedule::Animation)
            .width(100.0)
            .height(50.0))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = *host
        .elements
        .iter()
        .find(|(_, e)| e.kind == ElementKind::Shader)
        .unwrap()
        .0;
    with_animation(
        app.runtime_mut(),
        Animation::linear(Duration::from_secs(1)),
        |rt| {
            saved.borrow().as_ref().unwrap().set(rt, 1.0);
        },
    );
    host.flush(&mut app).unwrap();
    let content = host
        .presentation
        .element(id, Duration::from_millis(500))
        .unwrap()
        .canvas
        .unwrap();
    let commands = content
        .draw(Size::new(100.0, 50.0), Duration::ZERO)
        .unwrap();
    assert_eq!(
        f32::from_le_bytes(
            sample(&commands).0.uniform_bytes().unwrap()[..4]
                .try_into()
                .unwrap()
        ),
        0.5
    );
    assert_eq!(parents.get(), 1);
    assert_eq!(preparations.get(), 2);
    let mut cache = CanvasCache::default();
    let size = Size::new(100.0, 50.0);
    cache
        .render(&content, size, Duration::ZERO, true, false)
        .unwrap();
    let first = cache
        .render(&content, size, Duration::from_secs(1), true, false)
        .unwrap();
    assert_eq!(sample(&first).1, Duration::from_secs(1));
    cache.pause();
    let resumed = cache
        .render(&content, size, Duration::from_secs(100), true, false)
        .unwrap();
    assert_eq!(sample(&resumed).1, Duration::from_secs(1));
    let reduced = cache
        .render(&content, size, Duration::from_secs(101), true, true)
        .unwrap();
    assert_eq!(sample(&reduced).1, Duration::ZERO);
    assert!(app.prepare().unwrap().is_none());
}
#[test]
fn invalid_uniforms_fail_before_host_mutation() {
    let mut app = App::new(shader(program().uniform("typo", 2.0_f32)));
    assert!(app.prepare().err().unwrap().to_string().contains("typo"));
}
