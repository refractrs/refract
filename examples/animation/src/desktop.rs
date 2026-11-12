#[cfg(not(target_arch="wasm32"))]
fn main()->refract_native::Result<()> {
    refract_native::run(refract_native::NativeOptions {
        title:"Animation — Refract".into(),initial_url:None,width:800.0,height:680.0,
        background:refract_animation::BACKGROUND,diagnostics:false,
    }, |_|refract_animation::application())
}
#[cfg(target_arch="wasm32")]
fn main(){}
