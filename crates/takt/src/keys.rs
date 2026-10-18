use gpui_kit::base::actions::SelectDown;
use gpui_kit::{App, KeyBinding};

pub fn bind(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("j", SelectDown, Some("List"))]);
}
