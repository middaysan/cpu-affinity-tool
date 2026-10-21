//! Shared compact process-table presentation and snapshot-local sorting.
use super::shared_elements::{palette, COMPACT_TABLE_ROW_HEIGHT};
use eframe::egui::{self, Align, Layout, RichText};
use std::{cmp::Ordering, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    Name,
    Pid,
    Load,
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    pub column: Column,
    pub descending: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Order {
    pub sort: Option<Sort>,
    key: Option<u64>,
    sorted: Option<Sort>,
    pub rows: Arc<[usize]>,
}

impl Sort {
    pub fn compare(self, order: Ordering) -> Ordering {
        if self.descending {
            order.reverse()
        } else {
            order
        }
    }
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub fn load(self, a: Option<f32>, b: Option<f32>) -> Ordering {
        match (a.filter(|v| v.is_finite()), b.filter(|v| v.is_finite())) {
            (Some(a), Some(b)) => self.compare(a.total_cmp(&b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

impl Order {
    pub fn click(&mut self, column: Column) {
        self.sort = Some(match self.sort {
            Some(sort) if sort.column == column => Sort {
                column,
                descending: !sort.descending,
            },
            _ => Sort {
                column,
                descending: column == Column::Load,
            },
        });
    }
    pub fn prepare(
        &mut self,
        key: u64,
        count: usize,
        include: impl Fn(usize) -> bool,
        compare: impl Fn(usize, usize) -> Ordering,
    ) {
        if self.key == Some(key) && self.sorted == self.sort {
            return;
        }
        let mut rows: Vec<_> = (0..count).filter(|&i| include(i)).collect();
        rows.sort_unstable_by(|&a, &b| compare(a, b));
        self.rows = rows.into();
        self.key = Some(key);
        self.sorted = self.sort;
    }
}

pub(crate) fn name_cmp(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

pub(crate) fn cell(ui: &mut egui::Ui, width: f32, right: bool, text: RichText) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, COMPACT_TABLE_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(if right {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    }));
    child.add(egui::Label::new(text).truncate().selectable(false))
}

pub(crate) fn heading(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    column: Column,
    order: &mut Order,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, COMPACT_TABLE_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if response.clicked() {
        order.click(column);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    let active = order.sort.filter(|s| s.column == column);
    let right = matches!(column, Column::Pid | Column::Load);
    let colors = palette(ui);
    let color = if active.is_some() || response.hovered() || response.has_focus() {
        colors.text_primary
    } else {
        colors.text_muted
    };
    let text = ui
        .painter()
        .layout_no_wrap(title.into(), egui::FontId::proportional(9.0), color);
    let x = if right {
        rect.right() - text.size().x
    } else {
        rect.left()
    };
    ui.painter().galley(
        egui::pos2(x, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    if let Some(sort) = active {
        let center = egui::pos2(
            if right { x - 5.0 } else { rect.right() - 3.0 },
            rect.center().y,
        );
        let direction = if sort.descending { 1.0 } else { -1.0 };
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                center + egui::vec2(-2.5, -direction * 1.5),
                center + egui::vec2(2.5, -direction * 1.5),
                center + egui::vec2(0.0, direction * 2.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            2.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("Click to sort; click again to reverse.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_sort_toggles_and_unknown_always_follows_measured_values() {
        let mut order = Order::default();
        order.click(Column::Load);
        assert_eq!(
            order.sort,
            Some(Sort {
                column: Column::Load,
                descending: true
            })
        );
        let values = [None, Some(2.0), Some(11.0), Some(0.0)];
        let sort = order.sort.unwrap();
        order.prepare(
            1,
            4,
            |_| true,
            |a, b| sort.load(values[a], values[b]).then(a.cmp(&b)),
        );
        assert_eq!(&*order.rows, &[2, 1, 3, 0]);
        let same = order.rows.clone();
        order.prepare(
            1,
            4,
            |_| panic!("unchanged snapshot was scanned"),
            |_, _| panic!("unchanged snapshot was sorted"),
        );
        assert!(Arc::ptr_eq(&same, &order.rows));
        order.click(Column::Load);
        let sort = order.sort.unwrap();
        order.prepare(
            1,
            4,
            |_| true,
            |a, b| sort.load(values[a], values[b]).then(a.cmp(&b)),
        );
        assert_eq!(&*order.rows, &[3, 1, 2, 0]);
    }
    #[test]
    fn alphabetical_sort_ignores_case_and_new_snapshot_replaces_indices() {
        assert_eq!(name_cmp("Chrome.exe", "chrome.EXE"), Ordering::Equal);
        let mut order = Order::default();
        order.click(Column::Name);
        assert!(!order.sort.unwrap().descending);
        let names = ["z.exe", "Alpha.exe", "beta.exe"];
        order.prepare(1, 3, |_| true, |a, b| name_cmp(names[a], names[b]));
        assert_eq!(&*order.rows, &[1, 2, 0]);
        order.prepare(2, 3, |i| i != 1, |a, b| name_cmp(names[a], names[b]));
        assert_eq!(&*order.rows, &[2, 0]);
    }
}
