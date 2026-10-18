use chrono::{NaiveDate, TimeZone, Utc};
use serde_json::json;
use takt_google::model::{Item, TaskFields, TaskListFields};
use takt_store::*;

fn list(id: &str, title: &str) -> Item<TaskListFields> {
    Item::from_value(json!({"id": id, "title": title})).unwrap()
}

fn task(value: serde_json::Value) -> Item<TaskFields> {
    Item::from_value(value).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    store
        .apply_task_lists("acc", &[list("L1", "My Tasks")])
        .unwrap();
    store
}

#[test]
fn tasks_due_returns_dated_visible_tasks_in_range() {
    let mut store = seeded();
    store
        .apply_tasks_page("acc", "L1", &[
            task(json!({"id": "a", "title": "Pay rent", "status": "needsAction", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"})),
            task(json!({"id": "b", "title": "Done", "status": "completed", "due": "2026-10-21T00:00:00.000Z", "updated": "2026-10-17T11:00:00.000Z"})),
            task(json!({"id": "c", "title": "Cleared", "status": "completed", "hidden": true, "due": "2026-10-21T00:00:00.000Z"})),
            task(json!({"id": "e", "title": "Undated", "status": "needsAction"})),
            task(json!({"id": "f", "title": "Later", "status": "needsAction", "due": "2026-12-01T00:00:00.000Z"})),
        ], None)
        .unwrap();
    let got = store.tasks_due(d(2026, 10, 19), d(2026, 10, 26)).unwrap();
    let titles: Vec<_> = got.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["Pay rent", "Done"]);
    assert!(got[1].completed);
    assert_eq!(got[0].list_title, "My Tasks");
}

#[test]
fn deleted_task_is_removed() {
    let mut store = seeded();
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "a", "title": "x", "due": "2026-10-20T00:00:00.000Z"}),
            )],
            None,
        )
        .unwrap();
    let changes = store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(json!({"id": "a", "deleted": true}))],
            None,
        )
        .unwrap();
    assert!(
        store
            .tasks_due(d(2026, 10, 1), d(2026, 11, 1))
            .unwrap()
            .is_empty()
    );
    assert!(changes.task_lists.contains("L1"));
}

fn t(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 17, hour, 0, 0).unwrap()
}

#[test]
fn checkpoint_advances_and_never_goes_back() {
    let mut store = seeded();
    assert_eq!(store.task_checkpoint("acc", "L1").unwrap(), None);
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "a", "title": "x", "updated": "2026-10-17T11:00:00.000Z"}),
            )],
            Some(t(11)),
        )
        .unwrap();
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "b", "title": "y", "updated": "2026-10-17T09:00:00.000Z"}),
            )],
            Some(t(9)),
        )
        .unwrap();
    assert_eq!(store.task_checkpoint("acc", "L1").unwrap(), Some(t(11)));
}

#[test]
fn non_final_page_leaves_checkpoint_unchanged() {
    let mut store = seeded();
    store
        .apply_tasks_page("acc", "L1", &[], Some(t(8)))
        .unwrap();
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "a", "title": "x", "updated": "2026-10-17T11:00:00.000Z"}),
            )],
            None,
        )
        .unwrap();
    assert_eq!(store.task_checkpoint("acc", "L1").unwrap(), Some(t(8)));
}

#[test]
fn list_missing_from_full_list_is_removed_with_its_tasks() {
    let mut store = seeded();
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "a", "title": "x", "due": "2026-10-20T00:00:00.000Z"}),
            )],
            None,
        )
        .unwrap();
    store
        .apply_task_lists("acc", &[list("L2", "Errands")])
        .unwrap();
    assert!(
        store
            .tasks_due(d(2026, 10, 1), d(2026, 11, 1))
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.task_checkpoint("acc", "L2").unwrap(), None);
}

#[test]
fn reapplying_task_lists_keeps_checkpoint() {
    let mut store = seeded();
    store
        .apply_tasks_page(
            "acc",
            "L1",
            &[task(
                json!({"id": "a", "title": "x", "updated": "2026-10-17T11:00:00.000Z"}),
            )],
            Some(t(11)),
        )
        .unwrap();
    store
        .apply_task_lists("acc", &[list("L1", "Renamed")])
        .unwrap();
    assert!(store.task_checkpoint("acc", "L1").unwrap().is_some());
}
