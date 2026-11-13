//! OS services used by custom controls. These stay outside the renderer and the
//! portable application graph. Winit owns the native text-input client and event
//! translation; this adapter controls its focus session and candidate geometry.
//!
//! Winit 0.30 exposes preedit/commit, not surrounding-text queries or arbitrary
//! replacement ranges. This is a minimal IME bridge, not the full GPUI protocol.

use crate::renderer::Rect;
use refract_core::ElementId;
use winit::{
    dpi::{LogicalPosition, LogicalSize},
    window::Window,
};

pub(crate) trait Clipboard {
    fn read(&mut self) -> Option<String>;
    /// A failed write must never cause Cut to delete the selected text.
    fn write(&mut self, text: String) -> bool;
}

impl Clipboard for Option<arboard::Clipboard> {
    fn read(&mut self) -> Option<String> {
        if self.is_none() {
            *self = arboard::Clipboard::new().ok();
        }
        self.as_mut()?.get_text().ok()
    }

    fn write(&mut self, text: String) -> bool {
        if self.is_none() {
            *self = arboard::Clipboard::new().ok();
        }
        self.as_mut()
            .is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }
}

#[derive(Default)]
pub(crate) struct TextInputSession {
    target: Option<ElementId>,
}

impl TextInputSession {
    pub fn sync(&mut self, window: &Window, target: Option<ElementId>, area: Option<Rect>) {
        if self.target != target {
            // End the previous native composition session before focusing another
            // editor. Do not enable/disable IME on every paint.
            if self.target.is_some() {
                window.set_ime_allowed(false);
            }
            if target.is_some() {
                window.set_ime_allowed(true);
            }
            self.target = target;
        }
        if target.is_some() {
            if let Some(area) = area {
                window.set_ime_cursor_area(
                    LogicalPosition::new(area.x, area.y),
                    LogicalSize::new(area.width.max(1.0), area.height.max(1.0)),
                );
            }
        }
    }
}

/// Sample the desktop preference when painting or restoring a window. Other
/// native platforms currently have no preference bridge; browsers use media queries.
pub(crate) fn reduced_motion() -> bool {
    #[cfg(all(target_os = "macos", feature = "vello"))]
    { objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() }
    #[cfg(not(all(target_os = "macos", feature = "vello")))]
    { false }
}
