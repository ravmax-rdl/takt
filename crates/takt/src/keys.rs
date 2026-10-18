use gpui_kit::base::actions::SelectDown;
use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(takt, [JumpToToday, SyncNow, CloseDetail, PreviousRow]);

/// Key context set on the main view.
pub const CONTEXT: &str = "Takt";

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", SelectDown, Some("List")),
        // `k` goes through the main view so it can load earlier days at the top.
        KeyBinding::new("k", PreviousRow, Some(CONTEXT)),
        KeyBinding::new("t", JumpToToday, Some(CONTEXT)),
        KeyBinding::new("secondary-r", SyncNow, Some(CONTEXT)),
        KeyBinding::new("escape", CloseDetail, Some(CONTEXT)),
        // The list's own escape (Cancel) would clear the selection; this
        // deeper binding wins while the list is focused.
        KeyBinding::new("escape", CloseDetail, Some("Takt > List")),
    ]);
}
