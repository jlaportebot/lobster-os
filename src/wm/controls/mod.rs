//! Reusable immediate-mode controls (O4.1).
//!
//! A [`Ui`] wraps one window surface for one frame. Controls draw with the
//! text engine and theme colours, register hit regions whose tag is the
//! caller's `key`, and show hover / pressed / focus / disabled looks. The
//! apps that use them decode the key in their hit handler, so no per-app
//! router stage is needed. The pure parts (slider maths, dropdown state
//! machine, scroll maths) are separate modules with host tests.
//!
//! Used by Settings; designed for the Files app that comes next.

pub mod drag;
pub mod dropdown;
pub mod item_view;
pub mod rect;
pub mod scroll;
pub mod slider;
mod slider_draw;
pub mod toolbar;
pub mod ui;
mod widgets;

pub use drag::Track;
pub use dropdown::Dropdown;
pub use item_view::Column;
pub use rect::Rect;
pub use slider_draw::{accent_text, POPUP_ROW_H, PROGRESS_H, ROW_H, SHADOW_ALPHA};
pub use toolbar::CrumbItem;
pub use ui::{on_color, style, ControlState, Look, Ui, KEY_NONE};
pub use widgets::{button_width, ButtonKind, CONTROL_H, LABEL_PX, RADIUS, SWITCH_H, SWITCH_W};
