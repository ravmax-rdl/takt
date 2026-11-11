use gpui_kit::base::actions::SelectDown;
use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(
    takt,
    [
        JumpToToday,
        SyncNow,
        CloseDetail,
        PreviousRow,
        NextPeriod,
        PreviousPeriod,
        NextDay,
        PreviousDay,
        GridUp,
        GridDown,
        NextEvent,
        PreviousEvent,
        OpenSelected,
        ShowDay,
        ShowWeek,
        ShowMonth,
        ShowAgenda
    ]
);

/// Key context set on the main view.
pub const CONTEXT: &str = "Takt";
/// Key context of the day, week and month grids, inside [`CONTEXT`].
pub const GRID_CONTEXT: &str = "Grid";

pub fn bind(cx: &mut App) {
    let grid = Some("Takt > Grid");
    cx.bind_keys([
        // Everywhere in the main view.
        KeyBinding::new("t", JumpToToday, Some(CONTEXT)),
        KeyBinding::new("secondary-r", SyncNow, Some(CONTEXT)),
        KeyBinding::new("escape", CloseDetail, Some(CONTEXT)),
        KeyBinding::new("d", ShowDay, Some(CONTEXT)),
        KeyBinding::new("1", ShowDay, Some(CONTEXT)),
        KeyBinding::new("w", ShowWeek, Some(CONTEXT)),
        KeyBinding::new("2", ShowWeek, Some(CONTEXT)),
        KeyBinding::new("m", ShowMonth, Some(CONTEXT)),
        KeyBinding::new("3", ShowMonth, Some(CONTEXT)),
        KeyBinding::new("a", ShowAgenda, Some(CONTEXT)),
        KeyBinding::new("4", ShowAgenda, Some(CONTEXT)),
        KeyBinding::new("n", NextPeriod, Some(CONTEXT)),
        KeyBinding::new("p", PreviousPeriod, Some(CONTEXT)),
        // The agenda list. `k` goes through the main view so it can load
        // earlier days at the top; in a grid the same action steps back.
        KeyBinding::new("j", SelectDown, Some("List")),
        KeyBinding::new("k", PreviousRow, Some(CONTEXT)),
        // The list's own escape (Cancel) would clear the selection; this
        // deeper binding wins while the list is focused.
        KeyBinding::new("escape", CloseDetail, Some("Takt > List")),
        // The day, week and month grids.
        KeyBinding::new("j", NextPeriod, grid),
        KeyBinding::new("pagedown", NextPeriod, grid),
        KeyBinding::new("pageup", PreviousPeriod, grid),
        KeyBinding::new("left", PreviousDay, grid),
        KeyBinding::new("h", PreviousDay, grid),
        KeyBinding::new("right", NextDay, grid),
        KeyBinding::new("l", NextDay, grid),
        KeyBinding::new("up", GridUp, grid),
        KeyBinding::new("down", GridDown, grid),
        KeyBinding::new("tab", NextEvent, grid),
        KeyBinding::new("shift-tab", PreviousEvent, grid),
        KeyBinding::new("enter", OpenSelected, grid),
    ]);
}
