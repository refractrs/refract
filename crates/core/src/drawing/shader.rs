//! Application-owned WGSL programs and dedicated GPU drawing surfaces.

use super::{canvas_with, Animatable, Canvas, CanvasSchedule, DrawingInputs, Size};
use crate::{Color, Element, Handler, View, ViewContext, ViewError};
use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

/// Shader clocks follow the same visibility and reduced-motion policy as canvases.
pub type ShaderSchedule = CanvasSchedule;

/// A float scalar or vector matching a member of the WGSL `Uniforms` structure.
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderUniform {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
}

impl ShaderUniform {
    fn values(&self) -> &[f32] {
        match self {
            Self::Float(v) => std::slice::from_ref(v),
            Self::Vec2(v) => v,
            Self::Vec3(v) => v,
            Self::Vec4(v) => v,
        }
    }
    fn replace(&self, values: &[f64]) -> Self {
        let mut result = self.clone();
        let target: &mut [f32] = match &mut result {
            Self::Float(v) => std::slice::from_mut(v),
            Self::Vec2(v) => v,
            Self::Vec3(v) => v,
            Self::Vec4(v) => v,
        };
        for (target, value) in target.iter_mut().zip(values) {
            *target = value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
        }
        result
    }
}
impl From<f32> for ShaderUniform {
    fn from(value: f32) -> Self {
        Self::Float(value)
    }
}
impl From<[f32; 2]> for ShaderUniform {
    fn from(value: [f32; 2]) -> Self {
        Self::Vec2(value)
    }
}
impl From<[f32; 3]> for ShaderUniform {
    fn from(value: [f32; 3]) -> Self {
        Self::Vec3(value)
    }
}
impl From<[f32; 4]> for ShaderUniform {
    fn from(value: [f32; 4]) -> Self {
        Self::Vec4(value)
    }
}
impl From<Color> for ShaderUniform {
    fn from(value: Color) -> Self {
        Self::Vec4([value.0, value.1, value.2, value.3].map(|v| f32::from(v) / 255.0))
    }
}

/// Immutable straight-alpha sRGB RGBA8 image data. Clones share the pixels.
/// Decode encoded assets before constructing an image; no I/O occurs while drawing.
#[derive(Clone)]
pub struct ShaderImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}
impl ShaderImage {
    pub fn rgba(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Result<Self, ViewError> {
        let pixels = pixels.into();
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) * 4 != pixels.len() as u64
        {
            return Err(ViewError::new(
                "shader image must be RGBA8, nonempty, and at most 8192 x 8192",
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
impl PartialEq for ShaderImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}
impl fmt::Debug for ShaderImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Program {
    source: Arc<str>,
    members: BTreeMap<String, (usize, usize)>,
    uniform_size: usize,
}

/// A validated WGSL program with bound parameters. Keep the program in state or
/// a `LazyLock` and clone it when binding values, rather than parsing every frame.
///
/// Define `struct Uniforms` with f32/vec2f/vec3f/vec4f members and
/// `fn shade(frame: ShaderFrame) -> vec4<f32>`. Read parameters as `uniforms.name`.
/// Return straight-alpha sRGB. `sample_image(uv)` uses linear clamp-to-edge sampling.
#[derive(Clone, Debug)]
pub struct Shader {
    program: Arc<Program>,
    uniforms: BTreeMap<String, ShaderUniform>,
    image: Option<ShaderImage>,
    fallback: Color,
}
impl PartialEq for Shader {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.program, &other.program)
            && self.uniforms == other.uniforms
            && self.image == other.image
            && self.fallback == other.fallback
    }
}
impl Shader {
    /// Parse and validate application source on either host, including runtime strings.
    /// Uniforms omitted from Rust bindings default to zero. Resource bindings and
    /// GPU entry points are owned by Refract; user code supplies the `shade` function.
    pub fn wgsl(source: impl AsRef<str>) -> Result<Self, ViewError> {
        let source = format!(
            "{}\n// Application shader\n{}\n{}",
            include_str!("shader_header.wgsl"),
            source.as_ref(),
            include_str!("shader_entry.wgsl")
        );
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|e| ViewError::new(e.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|e| ViewError::new(e.emit_to_string(&source)))?;
        if module.entry_points.len() != 3
            || module
                .global_variables
                .iter()
                .filter(|(_, v)| v.binding.is_some())
                .count()
                != 4
        {
            return Err(ViewError::new(
                "shader resource bindings and entry points are reserved for Refract",
            ));
        }
        let ty = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Uniforms"))
            .map(|(_, ty)| ty)
            .ok_or_else(|| ViewError::new("shader must define struct Uniforms"))?;
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            return Err(ViewError::new("Uniforms must be a structure"));
        };
        if *span > 16384 {
            return Err(ViewError::new(
                "shader uniforms exceed the portable 16 KiB limit",
            ));
        }
        let mut fields = BTreeMap::new();
        for member in members {
            let count = match module.types[member.ty].inner {
                naga::TypeInner::Scalar(naga::Scalar {
                    kind: naga::ScalarKind::Float,
                    width: 4,
                }) => 1,
                naga::TypeInner::Vector {
                    size,
                    scalar:
                        naga::Scalar {
                            kind: naga::ScalarKind::Float,
                            width: 4,
                        },
                } => size as usize,
                _ => {
                    return Err(ViewError::new(
                        "shader uniforms support only f32 and f32 vectors",
                    ))
                }
            };
            fields.insert(
                member.name.clone().unwrap(),
                (member.offset as usize, count),
            );
        }
        Ok(Self {
            program: Arc::new(Program {
                source: source.into(),
                members: fields,
                uniform_size: (*span as usize).div_ceil(16) * 16,
            }),
            uniforms: BTreeMap::new(),
            image: None,
            fallback: Color(0, 0, 0, 0),
        })
    }
    /// Bind a named parameter. Names, dimensions and finite values are checked at mount.
    pub fn uniform(mut self, name: impl Into<String>, value: impl Into<ShaderUniform>) -> Self {
        self.uniforms.insert(name.into(), value.into());
        self
    }
    pub fn image(mut self, image: ShaderImage) -> Self {
        self.image = Some(image);
        self
    }
    /// Color shown by prerendering and browsers without a usable WebGPU device.
    pub fn fallback(mut self, color: Color) -> Self {
        self.fallback = color;
        self
    }
    pub fn fallback_color(&self) -> Color {
        self.fallback
    }
    pub fn image_input(&self) -> Option<&ShaderImage> {
        self.image.as_ref()
    }
    #[doc(hidden)]
    pub fn source(&self) -> &Arc<str> {
        &self.program.source
    }
    pub fn validate(&self) -> Result<(), ViewError> {
        for (name, value) in &self.uniforms {
            let Some((_, count)) = self.program.members.get(name) else {
                return Err(ViewError::new(format!("unknown shader uniform `{name}`")));
            };
            if *count != value.values().len() || value.values().iter().any(|v| !v.is_finite()) {
                return Err(ViewError::new(format!(
                    "shader uniform `{name}` requires {count} finite f32 components"
                )));
            }
        }
        Ok(())
    }
    #[doc(hidden)]
    pub fn uniform_bytes(&self) -> Result<Vec<u8>, ViewError> {
        self.validate()?;
        let mut bytes = vec![0; self.program.uniform_size];
        for (name, value) in &self.uniforms {
            let (offset, _) = self.program.members[name];
            for (i, value) in value.values().iter().enumerate() {
                bytes[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        Ok(bytes)
    }
    #[doc(hidden)]
    pub fn frame_bytes(size: Size, width: u32, height: u32, elapsed: Duration) -> Vec<u8> {
        [
            size.width as f32,
            size.height as f32,
            width as f32,
            height as f32,
            elapsed.as_secs_f32(),
            0.0,
            0.0,
            0.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect()
    }
}
impl Animatable for Shader {
    fn components(&self) -> Vec<f64> {
        self.uniforms
            .values()
            .flat_map(|v| v.values().iter().map(|v| f64::from(*v)))
            .collect()
    }
    fn with_components(&self, components: &[f64]) -> Self {
        let mut result = self.clone();
        let mut offset = 0;
        for value in result.uniforms.values_mut() {
            let count = value.values().len();
            *value = value.replace(&components[offset..offset + count]);
            offset += count;
        }
        result
    }
}

/// A bounded GPU surface with ordinary layout, events, and tracked inputs.
pub struct ShaderView {
    canvas: Canvas<Shader>,
}

pub fn shader(program: Shader) -> ShaderView {
    shader_with(move |_| program.clone())
}
/// Bind graph-dependent parameters at the shader mount, independently of its parent.
pub fn shader_with(prepare: impl Fn(&mut DrawingInputs<'_, '_>) -> Shader + 'static) -> ShaderView {
    ShaderView {
        canvas: canvas_with(prepare, |g, frame, shader| g.shader(shader.clone(), frame))
            .shader_kind()
            .validate_inputs(Shader::validate),
    }
}
impl crate::view::modifiers::ElementBuilder for ShaderView {
    fn element_mut(&mut self) -> &mut Element {
        self.canvas.element_mut()
    }
    fn handler_mut(&mut self) -> &mut Option<Handler> {
        self.canvas.handler_mut()
    }
}
impl ShaderView {
    crate::view::modifiers::element_modifiers!(pub Self, crate::view::modifiers::configure);
    crate::view::modifiers::event_modifiers!(pub Self, crate::view::modifiers::configure_handler);
    pub fn schedule(mut self, schedule: ShaderSchedule) -> Self {
        self.canvas = self.canvas.schedule(schedule);
        self
    }
    /// Bound uniform names and vector dimensions must remain stable while animating.
    pub fn animatable_inputs(mut self) -> Self {
        self.canvas = self.canvas.animatable_inputs();
        self
    }
}
impl View for ShaderView {
    fn body(&self, cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        // Validate before any host mutation, rather than deferring binding errors to paint.
        self.canvas.body(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "struct Uniforms { amount: f32, tint: vec3<f32>, offset: vec2<f32> }\nfn shade(frame: ShaderFrame) -> vec4<f32> { return vec4<f32>(uniforms.tint * uniforms.amount, 1.0); }";

    #[test]
    fn packs_reflected_offsets_and_rejects_invalid_bindings() {
        let shader = Shader::wgsl(SOURCE)
            .unwrap()
            .uniform("amount", 0.5_f32)
            .uniform("tint", [1.0, 0.25, 0.75])
            .uniform("offset", [2.0, 3.0]);
        let bytes = shader.uniform_bytes().unwrap();
        let value = |offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(bytes.len(), 48);
        assert_eq!(value(0), 0.5);
        assert_eq!(&bytes[4..16], &[0; 12]);
        assert_eq!(value(16), 1.0);
        assert_eq!(value(32), 2.0);
        assert!(shader
            .clone()
            .uniform("missing", 1.0_f32)
            .validate()
            .unwrap_err()
            .to_string()
            .contains("missing"));
        assert!(shader
            .clone()
            .uniform("tint", [1.0, 2.0])
            .validate()
            .is_err());
        assert!(shader.uniform("amount", f32::NAN).validate().is_err());
    }

    #[test]
    fn validates_custom_source_and_resource_contract() {
        assert!(Shader::wgsl("invalid WGSL").is_err());
        assert!(Shader::wgsl(SOURCE.replace("vec3<f32>", "vec3<u32>")).is_err());
        let extra = format!("{SOURCE}\n@group(1) @binding(0) var extra: texture_2d<f32>;");
        assert!(Shader::wgsl(extra)
            .unwrap_err()
            .to_string()
            .contains("reserved"));
        assert!(ShaderImage::rgba(2, 2, vec![0; 15]).is_err());
        assert!(ShaderImage::rgba(0, 0, vec![]).is_err());
    }
}
