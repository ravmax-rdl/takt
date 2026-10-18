use chrono::NaiveDate;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agenda::{AgendaRow, DaySection, RowKey};
use crate::model::CalendarModel;

pub struct AgendaDelegate {
    pub model: Entity<CalendarModel>,
    pub sections: Vec<DaySection>,
    pub selected: Option<IndexPath>,
}

impl AgendaDelegate {
    pub fn row(&self, ix: IndexPath) -> Option<&AgendaRow> {
        self.sections.get(ix.section)?.rows.get(ix.row)
    }

    /// The day a row sits under. A multi-day event has one row per day, all
    /// with the same key, so the day is part of a row's identity.
    pub fn date_of(&self, ix: IndexPath) -> Option<NaiveDate> {
        self.sections.get(ix.section).map(|s| s.date)
    }

    pub fn index_of(&self, date: NaiveDate, key: &RowKey) -> Option<IndexPath> {
        find_row(&self.sections, date, key)
    }

    // Used from Task 19 on, where moving up from the first row loads earlier days.
    #[allow(dead_code)]
    pub fn is_first(&self, ix: IndexPath) -> bool {
        ix.section == 0 && ix.row == 0
    }
}

/// Finds `key` under `date`. When that day no longer shows the row (the event
/// moved or shrank), picks the nearest day that does.
pub fn find_row(sections: &[DaySection], date: NaiveDate, key: &RowKey) -> Option<IndexPath> {
    sections
        .iter()
        .enumerate()
        .filter_map(|(s, section)| {
            let r = section.rows.iter().position(|r| r.key == *key)?;
            Some((
                (section.date - date).num_days().abs(),
                IndexPath::new(r).section(s),
            ))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, ix)| ix)
}

fn load_earlier_button(model: Entity<CalendarModel>) -> Button {
    Button::new("load-earlier")
        .ghost()
        .xsmall()
        .label("Load earlier")
        .on_click(move |_, _, cx| model.update(cx, |m, cx| m.load_earlier(cx)))
}

impl ListDelegate for AgendaDelegate {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.sections.len().max(1)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.sections.get(section).map_or(0, |s| s.rows.len())
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let header = self.sections.get(section)?.header.clone();
        let model = self.model.clone();
        Some(
            h_flex()
                .w_full()
                .px_3()
                .pt_3()
                .pb_1()
                .justify_between()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(header)
                .when(section == 0, |this| this.child(load_earlier_button(model))),
        )
    }

    /// With no rows the list shows neither headers nor scroll-triggered
    /// loading, so both directions get a button here.
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let model = self.model.clone();
        let later = self.model.clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("Nothing scheduled in this range.")
            .child(
                h_flex().gap_2().child(load_earlier_button(model)).child(
                    Button::new("load-later")
                        .ghost()
                        .xsmall()
                        .label("Load later")
                        .on_click(move |_, _, cx| later.update(cx, |m, cx| m.load_later(cx))),
                ),
            )
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let row = self.row(ix)?.clone();
        Some(
            ListItem::new(("agenda-row", ix.section * 100_000 + ix.row))
                .selected(self.selected == Some(ix))
                .child(render_row(&row, cx)),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
        cx.notify();
    }

    fn has_more(&self, _: &App) -> bool {
        true
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.model.update(cx, |m, cx| m.load_later(cx));
    }
}

/// One line per row: gpui-kit's list needs every item the same height.
fn render_row(row: &AgendaRow, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let color = rgb(row.color);
    let title = match &row.location {
        Some(location) => format!("{}  ·  {location}", row.title),
        None => row.title.clone(),
    };
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .text_sm()
        .child(
            div()
                .w(px(132.))
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(row.time_label.clone()),
        )
        .child(if row.is_task {
            div()
                .flex_none()
                .text_color(color)
                .child(if row.completed { "☑" } else { "☐" })
        } else {
            div().flex_none().size(px(8.)).rounded_full().bg(color)
        })
        .child(
            div()
                .flex_1()
                .truncate()
                .when(row.declined || row.completed, |d| {
                    d.line_through().text_color(theme.muted_foreground)
                })
                .child(title),
        )
        .when(row.warning, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.danger)
                    .child("rule not understood"),
            )
        })
        .when_some(row.account_badge.clone(), |this, badge| {
            this.child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(badge),
            )
        })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use chrono::NaiveDate;

    use super::find_row;
    use crate::agenda::{AgendaRow, DaySection, RowKey};

    fn key(id: &str) -> RowKey {
        RowKey::Event {
            account_id: "a".into(),
            calendar_id: "c".into(),
            event_id: id.into(),
            instance_id: id.into(),
        }
    }

    fn row(id: &str) -> AgendaRow {
        AgendaRow {
            key: key(id),
            time_label: String::new(),
            title: id.into(),
            location: None,
            color: 0,
            account_badge: None,
            declined: false,
            is_task: false,
            completed: false,
            warning: false,
        }
    }

    fn day(d: u32, rows: &[&str]) -> DaySection {
        DaySection {
            date: NaiveDate::from_ymd_opt(2026, 10, d).unwrap(),
            header: String::new(),
            rows: rows.iter().map(|id| row(id)).collect(),
        }
    }

    #[test]
    fn multi_day_event_keeps_the_selected_day() {
        let sections = vec![
            day(1, &["x", "trip"]),
            day(2, &["trip"]),
            day(3, &["y", "trip"]),
        ];
        let ix = find_row(&sections, sections[2].date, &key("trip")).unwrap();
        assert_eq!((ix.section, ix.row), (2, 1));
    }

    #[test]
    fn row_gone_from_its_day_falls_back_to_nearest_day() {
        let sections = vec![
            day(1, &["trip"]),
            day(2, &["y"]),
            day(4, &["trip"]),
            day(5, &["y"]),
        ];
        let ix = find_row(&sections, sections[3].date, &key("trip")).unwrap();
        assert_eq!((ix.section, ix.row), (2, 0));
        assert!(find_row(&sections, sections[0].date, &key("gone")).is_none());
    }
}
