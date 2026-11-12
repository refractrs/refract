//! Insertion/removal effects. Logical unmount is immediate; hosts retain inert visuals.
use crate::{Style, Transform, ViewError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionEffect {
    pub opacity: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}
impl Default for TransitionEffect {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
        }
    }
}
impl TransitionEffect {
    pub(crate) fn combined(self, other: Self) -> Self {
        Self {
            opacity: self.opacity * other.opacity,
            offset_x: self.offset_x + other.offset_x,
            offset_y: self.offset_y + other.offset_y,
            scale: self.scale * other.scale,
        }
    }
    pub fn at(self, progress: f32) -> Self {
        Self {
            opacity: (1.0 + (self.opacity - 1.0) * progress).clamp(0.0, 1.0),
            offset_x: self.offset_x * progress,
            offset_y: self.offset_y * progress,
            scale: 1.0 + (self.scale - 1.0) * progress,
        }
    }
    pub fn transform(self) -> Transform {
        Transform {
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            scale_x: self.scale,
            scale_y: self.scale,
            ..Default::default()
        }
    }
    pub(crate) fn apply(self, style: &mut Style) {
        style.opacity = Some(style.opacity.unwrap_or(1.0) * self.opacity);
        style.transform.offset_x += self.offset_x;
        style.transform.offset_y += self.offset_y;
        style.transform.scale_x *= self.scale;
        style.transform.scale_y *= self.scale;
    }
    fn validate(self) -> Result<(), ViewError> {
        self.transform().validate()?;
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(ViewError::new(
                "transition opacity must be between zero and one",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transition {
    pub insertion: TransitionEffect,
    pub removal: TransitionEffect,
}
impl Transition {
    pub fn identity() -> Self {
        Self::default()
    }
    pub fn opacity() -> Self {
        Self::symmetric(TransitionEffect {
            opacity: 0.0,
            ..Default::default()
        })
    }
    pub fn offset(x: f32, y: f32) -> Self {
        Self::symmetric(TransitionEffect {
            offset_x: x,
            offset_y: y,
            ..Default::default()
        })
    }
    pub fn scale(scale: f32) -> Self {
        Self::symmetric(TransitionEffect {
            scale,
            ..Default::default()
        })
    }
    fn symmetric(effect: TransitionEffect) -> Self {
        Self {
            insertion: effect,
            removal: effect,
        }
    }
    pub fn combined_with(self, other: Self) -> Self {
        Self {
            insertion: self.insertion.combined(other.insertion),
            removal: self.removal.combined(other.removal),
        }
    }
    pub fn asymmetric(insertion: Self, removal: Self) -> Self {
        Self {
            insertion: insertion.insertion,
            removal: removal.removal,
        }
    }
    pub fn validate(self) -> Result<(), ViewError> {
        self.insertion.validate()?;
        self.removal.validate()
    }
}
