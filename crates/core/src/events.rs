//! Portable input payloads and event-dispatch results.
use crate::{ElementId, ScrollEvent};

/// Keyboard modifier keys held when an input event occurs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Whether a keyboard key was pressed or released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// A key press, including repeated presses while held.
    Down,
    /// A key release.
    Up,
}

/// A logical keyboard key, interpreted using the active keyboard layout.
/// Use [`KeyEvent::code`] when the physical key position matters instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// Text produced by a character key.
    Character(String),
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    /// Other platform key names, including function and modifier keys.
    Named(String),
}

impl Key {
    /// Translate a logical key name (using browser KeyboardEvent.key spelling).
    pub fn from_name(name: &str) -> Self {
        match name {
            "Enter" => Self::Enter,
            "Escape" => Self::Escape,
            "Tab" => Self::Tab,
            " " | "Space" => Self::Space,
            "Backspace" => Self::Backspace,
            "Delete" => Self::Delete,
            "ArrowLeft" => Self::ArrowLeft,
            "ArrowRight" => Self::ArrowRight,
            "ArrowUp" => Self::ArrowUp,
            "ArrowDown" => Self::ArrowDown,
            "Home" => Self::Home,
            "End" => Self::End,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            value if value.chars().count() == 1 => Self::Character(value.into()),
            value => Self::Named(value.into()),
        }
    }
}

/// A keyboard event with logical and physical keys, modifiers, and composition state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    /// Physical key name, independent of the keyboard layout (for example KeyA).
    pub code: String,
    pub state: KeyState,
    pub modifiers: Modifiers,
    pub repeat: bool,
    /// Shortcuts should normally ignore keys while an IME is composing text.
    pub is_composing: bool,
}

/// The kind of pointing device that produced a pointer event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// A mouse or a device reported as a mouse, such as a trackpad.
    Mouse,
    /// A touch contact.
    Touch,
    /// A pen or stylus.
    Pen,
    /// A device whose kind is not identified by the host.
    Unknown,
}

/// The stage of a pointer interaction, including movement, cancellation, and boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    /// A button is pressed or a contact begins.
    Down,
    /// The pointer moves.
    Move,
    /// A button is released or a contact ends.
    Up,
    /// The host cancels the interaction before normal completion.
    Cancel,
    /// The pointer enters an element's interaction region.
    Enter,
    /// The pointer leaves an element's interaction region.
    Leave,
}

/// A pointer button identified by its role rather than its physical placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// The primary activation button, usually the left mouse button.
    Primary,
    /// The secondary button, usually the right mouse button.
    Secondary,
    /// The middle mouse button.
    Middle,
    /// The navigation-back button.
    Back,
    /// The navigation-forward button.
    Forward,
    /// Another button, identified by its bit index in [`PointerButtons`].
    Other(u16),
}

/// Buttons currently held, using Pointer Events bit assignments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerButtons(pub u16);
impl PointerButtons {
    pub const PRIMARY: u16 = 1;
    pub const SECONDARY: u16 = 2;
    pub const MIDDLE: u16 = 4;
    pub fn contains(self, button: PointerButton) -> bool {
        self.0 & button.mask() != 0
    }
}
impl PointerButton {
    pub fn mask(self) -> u16 {
        match self {
            Self::Primary => 1,
            Self::Secondary => 2,
            Self::Middle => 4,
            Self::Back => 8,
            Self::Forward => 16,
            Self::Other(n) => 1u16.checked_shl(n.into()).unwrap_or(0),
        }
    }
}

/// Logical pixels relative to the mounting surface (the desktop window or web root).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventPoint {
    pub x: f64,
    pub y: f64,
}

/// A pointer event with device identity, position, buttons, modifiers, and pressure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub id: u64,
    pub kind: PointerKind,
    pub phase: PointerPhase,
    pub position: EventPoint,
    pub button: Option<PointerButton>,
    pub buttons: PointerButtons,
    pub modifiers: Modifiers,
    pub pressure: f32,
}

/// Key and pointer down/move/up/cancel events bubble through mounted ancestors.
/// Activation, text, scroll, focus, hover, and pointer enter/leave target one element.
#[derive(Clone, Debug)]
pub enum Event {
    Press,
    Input(String),
    Submit,
    Key(KeyEvent),
    Pointer(PointerEvent),
    Focus(bool),
    /// Targeted notification: this element or a descendant owns focus.
    FocusWithin(bool),
    Hover(bool),
    Scroll(ScrollEvent),
}
impl Event {
    pub(crate) fn bubbles(&self) -> bool {
        matches!(
            self,
            Self::Key(_)
                | Self::Pointer(PointerEvent {
                    phase: PointerPhase::Down
                        | PointerPhase::Move
                        | PointerPhase::Up
                        | PointerPhase::Cancel,
                    ..
                })
        )
    }
}

/// A request for the host to acquire or release pointer capture for an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerCapture {
    pub pointer_id: u64,
    pub target: ElementId,
    pub capture: bool,
}

/// Synchronous result consumed by hosts before applying native default behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventResponse {
    pub handled: bool,
    pub default_prevented: bool,
    pub propagation_stopped: bool,
    pub pointer_capture: Option<PointerCapture>,
}
