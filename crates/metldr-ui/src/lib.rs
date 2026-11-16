//! The behaviour layer every metldr control is built from: each module is one primitive with its
//! states, keyboard model and accessibility, rendered with Rune. The behaviours follow a
//! published Apache-2.0 component library; `LICENSE-APACHE` beside this crate covers them.

pub mod announcer;
pub mod button;
pub mod choice;
pub mod collapsible;
pub mod dialog;
pub mod field;
pub mod menu;
pub mod popup;
pub mod press;
pub mod scroll_area;
pub mod tabs;
pub mod toggle;
pub mod tooltip;
