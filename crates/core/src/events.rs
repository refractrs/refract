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

/// A directional text range expressed in UTF-16 code units on every host.
/// Hosts clamp requested offsets to valid character boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: u32,
    pub focus: u32,
}

impl TextSelection {
    pub const fn caret(offset: u32) -> Self {
        Self {
            anchor: offset,
            focus: offset,
        }
    }
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }
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

/// Units supplied by the wheel device. Trackpads normally report logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelUnit {
    Pixel,
    Line,
    Page,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Wheel input before scrolling. Positive deltas scroll right/down on both hosts.
/// Browser wheel events have no gesture phase. Preventing the default suppresses
/// scrolling for this event without changing the scroll view's saved position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelEvent {
    pub position: EventPoint,
    pub delta: crate::ScrollOffset,
    pub unit: WheelUnit,
    pub phase: Option<WheelPhase>,
    pub modifiers: Modifiers,
}

/// Text representations exchanged by clipboard commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardContent {
    pub text: String,
    pub html: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardCommand {
    Copy,
    Cut,
    Paste,
}

/// A host clipboard command. Paste supplies content; Copy/Cut handlers provide
/// content with `set_content` and prevent the host default. The shared payload
/// lets hosts synchronously populate browser clipboard events without async I/O.
#[derive(Clone, Debug)]
pub struct ClipboardEvent {
    pub command: ClipboardCommand,
    content: std::rc::Rc<std::cell::RefCell<Option<ClipboardContent>>>,
}

impl ClipboardEvent {
    pub fn new(command: ClipboardCommand, content: Option<ClipboardContent>) -> Self {
        Self {
            command,
            content: std::rc::Rc::new(std::cell::RefCell::new(content)),
        }
    }

    pub fn content(&self) -> Option<ClipboardContent> {
        self.content.borrow().clone()
    }

    pub fn set_content(&self, content: ClipboardContent) {
        *self.content.borrow_mut() = Some(content);
    }
}

/// Key, pointer down/move/up/cancel, wheel, and clipboard events bubble through mounted ancestors.
/// Activation, text, scroll, focus, hover, and pointer enter/leave target one element.
#[derive(Clone, Debug)]
pub enum Event {
    /// Semantic activation, such as a click or keyboard activation of a button.
    Press,
    /// User edit carrying the control's complete text value; model writes do not emit this.
    Input(String),
    /// Current UTF-16 selection, observed on changes and synchronized before commands.
    TextSelection(TextSelection),
    /// Text control submission, such as Enter in a single-line field.
    Submit,
    /// Keyboard press or release with modifiers, repeat, and composition state.
    Key(KeyEvent),
    /// Pointer interaction with its phase, position, buttons, and modifiers.
    Pointer(PointerEvent),
    /// Direct focus change; true when this element gains focus, false when it loses focus.
    Focus(bool),
    /// Subtree focus change; true while this element or a descendant owns focus.
    FocusWithin(bool),
    /// Hover change; true when the pointer enters the element, false when it leaves.
    Hover(bool),
    /// Observed scroll position and viewport/content metrics from the host.
    Scroll(ScrollEvent),
    /// Wheel input before host scrolling, with deltas and their units.
    Wheel(WheelEvent),
    /// Copy, cut, or paste command with shared clipboard content for synchronous handling.
    Clipboard(ClipboardEvent),
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
                | Self::Wheel(_)
                | Self::Clipboard(_)
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
