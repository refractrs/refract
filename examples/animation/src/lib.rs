//! Shared animation playground: the same curves, springs and transitions on both hosts.
use refract::prelude::*;
use std::time::Duration;
const INK:Color=Color::rgb(28,43,61);
const ACCENT:Color=Color::rgb(40,87,183);
pub const BACKGROUND:Color=Color::rgb(248,250,253);

fn control(label:&str, action:impl Fn(&mut ActionContext<'_>)+'static) -> impl View {
    text(label).font_size(13.0).padding(12.0).background(ACCENT).foreground(Color::WHITE).radius(8.0).on_press(action).label(label)
}

#[view]
fn lane(cx:&mut ViewContext<'_>, value:Binding<bool>, label:String, animation:Animation) -> impl View {
    let selected=cx.read(&value);
    v_stack().gap(10.0)
        .child(text(&label).font_size(12.0).foreground(INK))
        .child(view().width(40.0).height(40.0).radius(if selected {20.0} else {8.0})
            .background(if selected {Color::rgb(45,145,112)} else {ACCENT})
            .offset(if selected {220.0} else {0.0},0.0)
            .rotation_effect(if selected {180.0} else {0.0})
            .label(label).animation(animation,selected))
        .padding(12.0).width(300.0).background(Color::WHITE).radius(10.0)
}

#[view]
pub fn application(cx:&mut ViewContext<'_>) -> impl View {
    let moved=cx.state("moved",||false);
    let visible=cx.state("visible",||true);
    let expanded=cx.state("expanded",||false);
    let show=cx.read(&visible); let open=cx.read(&expanded);
    let toggle=moved.clone(); let toggle_visible=visible.clone();let toggle_size=expanded.clone();
    v_stack().gap(20.0)
        .child(text("Motion, with intent.").font_size(30.0).font_weight(600))
        .child(text("Reverse while moving to compare easing and spring continuity.").font_size(13.0))
        .child(h_stack().gap(10.0)
            .child(control("Reverse motion",move |cx|toggle.update(cx.runtime(),|v|*v=!*v)))
            .child(control("Toggle notice",move |cx|with_animation(cx.runtime(),Animation::ease_out(Duration::from_millis(400)),|rt|toggle_visible.update(rt,|v|*v=!*v))))
            .child(control("Resize panel",move |cx|toggle_size.update(cx.runtime(),|v|*v=!*v))))
        .child(h_stack().gap(16.0)
            .child(lane(moved.clone(),"Easing".into(),Animation::ease_in_out(Duration::from_millis(650))))
            .child(lane(moved,"Spring".into(),Animation::spring_with(Duration::from_millis(550),0.65))))
        .child(view().height(if open {130.0} else {60.0}).fill_width().background(Color::rgb(226,235,250)).radius(12.0)
            .child(text("Explicit dimensions animate layout.").padding(18.0))
            .animation(Animation::ease_in_out(Duration::from_millis(350)),open))
        .child(if show {text("Notice: removal releases input and effects immediately.").font_size(13.0).padding(16.0)
            .fill_width().background(Color::rgb(225,242,235)).radius(10.0)
            .transition(Transition::opacity().combined_with(Transition::offset(0.0,8.0)))
            .label("Animated notice").into_any()} else {Empty.into_any()})
        .padding(28.0).width(720.0).foreground(INK)
}

#[cfg(target_arch="wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    pub struct AnimationApp(refract_web::Mount);
    #[wasm_bindgen]
    impl AnimationApp {
        pub fn dispose(&self){self.0.dispose()}
        pub fn revision(&self)->u64{self.0.revision()}
        pub fn dom_calls(&self)->u64{self.0.dom_calls()}
        pub fn last_error(&self)->Option<String>{self.0.last_error()}
    }
    #[wasm_bindgen]
    pub fn start(host:web_sys::Element)->Result<AnimationApp,JsValue>{
        Ok(AnimationApp(refract_web::mount(host,application(),Default::default())?))
    }
}
