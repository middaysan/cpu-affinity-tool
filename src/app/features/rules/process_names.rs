use eframe::egui;
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

pub(crate) fn matches(names: &[String], query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<_> = names
        .iter()
        .filter_map(|n| {
            let key = n.to_lowercase();
            key.contains(&query)
                .then(|| (!key.starts_with(&query), key, n.clone()))
        })
        .collect();
    matches.sort_unstable();
    matches.dedup_by(|a, b| a.1 == b.1);
    matches
        .into_iter()
        .take(8)
        .map(|(_, _, name)| name)
        .collect()
}
#[derive(Default)]
pub(crate) struct ProcessNames {
    pub names: Arc<Vec<String>>,
    pending: Option<mpsc::Receiver<Vec<String>>>,
    sampled: Option<Instant>,
    requested: bool,
}
impl ProcessNames {
    pub fn request_refresh(&mut self) {
        if self.pending.is_none()
            && self
                .sampled
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
        {
            self.requested = true;
        }
    }
    pub fn prepare(&mut self, ctx: &egui::Context, allow_native: bool) {
        if let Some(names) = self.pending.as_ref().and_then(|r| r.try_recv().ok()) {
            self.names = Arc::new(names);
            self.pending = None;
            self.sampled = Some(Instant::now());
        }
        if !allow_native
            || !self.requested
            || self.pending.is_some()
            || self
                .sampled
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.requested = false;
        let (tx, rx) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        self.pending = Some(rx);
        if std::thread::Builder::new()
            .name("process-name-suggestions".into())
            .spawn(move || {
                let mut names: Vec<_> = os_api::OS::get_all_process_names()
                    .into_iter()
                    .take(65536)
                    .map(|(_, n)| n)
                    .filter(|n| !n.is_empty() && n.len() <= 1024)
                    .collect();
                names.sort_unstable_by_key(|n| (n.to_lowercase(), n.clone()));
                names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                let _ = tx.send(names);
                wake.request_repaint();
            })
            .is_err()
        {
            self.pending = None;
            self.sampled = Some(Instant::now());
        }
    }
}
#[derive(Clone)]
struct Suggestions {
    source: Arc<Vec<String>>,
    query: String,
    rows: Vec<String>,
}
pub(crate) fn field(
    ui: &mut egui::Ui,
    name: &mut String,
    width: f32,
    names: &Arc<Vec<String>>,
    _index: usize,
) -> bool {
    let edit = ui.add(
        crate::app::shell::presenters::shared_elements::form_text_edit(name).desired_width(width),
    );
    crate::debug_mcp_control!(format!("rule.process_name.{_index}"), edit);
    let id = edit.id.with("process-name-suggestions");
    let cache_id = id.with("matches");
    let mut changed = edit.changed();
    if !edit.has_focus() && !egui::Popup::is_id_open(ui.ctx(), id) {
        ui.ctx()
            .data_mut(|d| d.remove::<Arc<Suggestions>>(cache_id));
        return changed;
    }
    let mut suggestions = ui.ctx().data(|d| d.get_temp::<Arc<Suggestions>>(cache_id));
    let refreshed = suggestions
        .as_ref()
        .is_none_or(|s| !Arc::ptr_eq(&s.source, names) || s.query != *name);
    if refreshed {
        let next = Arc::new(Suggestions {
            source: names.clone(),
            query: name.clone(),
            rows: matches(names, name)
                .into_iter()
                .filter(|n| !n.eq_ignore_ascii_case(name.trim()))
                .collect(),
        });
        ui.ctx().data_mut(|d| d.insert_temp(cache_id, next.clone()));
        suggestions = Some(next);
    }
    if edit.gained_focus() || edit.changed() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("process-names-refresh"), true));
    }
    let suggestions = suggestions.unwrap();
    if suggestions.rows.is_empty() {
        egui::Popup::close_id(ui.ctx(), id);
        return changed;
    }
    if edit.gained_focus() || edit.changed() || (refreshed && edit.has_focus()) {
        egui::Popup::open_id(ui.ctx(), id);
    }
    let mut selected = None;
    egui::Popup::from_response(&edit)
        .id(id)
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .width(edit.rect.width())
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .show(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.label(egui::RichText::new("Running processes").small().weak());
            for (_i, item) in suggestions.rows.iter().enumerate() {
                let row = ui.push_id(_i, |ui| ui.selectable_label(false, item)).inner;
                crate::debug_mcp_control!(format!("rule.process_suggestion.{_index}.{_i}"), row);
                if crate::app::shell::presenters::shared_elements::wrapped_tooltip(row, item)
                    .clicked()
                {
                    selected = Some(item.clone());
                    ui.close();
                }
            }
        });
    if let Some(selected) = selected {
        *name = selected;
        changed = true;
        egui::Popup::close_id(ui.ctx(), id);
        edit.request_focus();
    }
    changed
}
#[cfg(test)]
mod tests {
    #[test]
    fn suggestion_click_fills_the_field_and_escape_preserves_free_text() {
        use eframe::egui::{self, Pos2, RawInput, Rect};
        use std::sync::Arc;
        let ctx = egui::Context::default();
        crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
        let names = Arc::new(vec![
            "chrome.exe".to_string(),
            "chrome_helper.exe".to_string(),
        ]);
        let mut name = "ch".to_string();
        let frame = |name: &mut String, events| {
            let mut changed = false;
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))),
                    events,
                    ..Default::default()
                },
                |ui| changed = super::field(ui, name, 240.0, &names, 0),
            );
            output.textures_delta.clear();
            (output, changed)
        };
        frame(&mut name, vec![]);
        frame(&mut name, vec![]);
        let click = |pos: Pos2| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&mut name, click(egui::pos2(35.0, 17.0)));
        let mut output = frame(&mut name, vec![]).0;
        for _ in 0..3 {
            output = frame(&mut name, vec![]).0;
        }
        let point = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "chrome.exe" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Running process suggestions must be visible");
        assert_eq!(name, "ch");
        let (_, changed) = frame(&mut name, click(point));
        assert!(changed);
        assert_eq!(name, "chrome.exe");
        frame(
            &mut name,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(name, "chrome.exe");
        let mut cache = super::ProcessNames::default();
        cache.prepare(&ctx, true);
        assert!(
            cache.pending.is_none(),
            "Idle painting must not enumerate native processes"
        );
    }
    #[test]
    fn suggestions_match_case_insensitively_and_rank_prefixes_without_duplicates() {
        let names = vec![
            "mychrome.exe".into(),
            "Chrome.exe".into(),
            "chrome.exe".into(),
            "chrome_helper.exe".into(),
            "notepad.exe".into(),
        ];
        assert_eq!(
            super::matches(&names, " CHRO "),
            vec!["Chrome.exe", "chrome_helper.exe", "mychrome.exe"]
        );
        assert!(super::matches(&names, " ").is_empty());
        assert!(super::matches(&names, "no-match").is_empty());
        assert_eq!(
            super::matches(
                &(0..100).map(|i| format!("app{i}.exe")).collect::<Vec<_>>(),
                "app"
            )
            .len(),
            8
        );
    }
}
