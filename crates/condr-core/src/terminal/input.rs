use super::{TerminalPosition, TerminalSelection, TerminalSelectionUnit, TerminalSize};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseWheel {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalMousePosition {
    pub row: u16,
    pub column: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseEvent {
    Button {
        button: TerminalMouseButton,
        pressed: bool,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
    Motion {
        button: Option<TerminalMouseButton>,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
    Wheel {
        direction: TerminalMouseWheel,
        amount: u16,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalKey {
    Character(String),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
}

/// What happened to a key. Only a program that asked for kitty event types tells a
/// repeat from a press or sees a release.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalKeyEventKind {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalScroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalCommand {
    Key {
        key: TerminalKey,
        modifiers: TerminalModifiers,
        kind: TerminalKeyEventKind,
    },
    Text(String),
    Paste(String),
    Mouse(TerminalMouseEvent),
    Focus(bool),
    Resize(TerminalSize),
    Scroll(TerminalScroll),
    /// Replaces the Server-tracked selection; `None` clears it.
    Select(Option<TerminalSelection>),
    /// Replaces the Server-tracked selection with the word or line around a viewport cell.
    SelectAt {
        position: TerminalPosition,
        display_offset: u32,
        unit: TerminalSelectionUnit,
    },
    /// Copies the Server-tracked selection, or an explicit viewport range.
    Copy {
        selection: Option<TerminalSelection>,
    },
}
