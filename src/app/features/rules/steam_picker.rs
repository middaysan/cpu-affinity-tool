use eframe::egui;
use std::{path::PathBuf, sync::mpsc};

#[derive(Default)]
pub(crate) struct SteamProcessPicker {
    open: bool,
    query: String,
    rows: Vec<os_api::steam::GameProcess>,
    pending: Option<mpsc::Receiver<(u64, String, Vec<os_api::steam::GameProcess>)>>,
    epoch: u64,
    requested: Option<String>,
    edited: Option<std::time::Instant>,
}
impl SteamProcessPicker {
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.epoch == u64::MAX {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let query = self.query.clone();
        let epoch = self.epoch;
        self.requested = Some(query.clone());
        self.pending = Some(rx);
        if std::thread::Builder::new()
            .name("steam-process-picker".into())
            .spawn(move || {
                let rows = os_api::steam::process_choices(&query);
                let _ = tx.send((epoch, query, rows));
                wake.request_repaint();
            })
            .is_err()
        {
            self.pending = None;
            self.requested = None;
        }
    }
    pub(crate) fn reset_view(&mut self) {
        // Keep receiver ownership until the finite worker exits; closing/reopening
        // the editor cannot create concurrent native enumerations.
        self.epoch = self.epoch.saturating_add(1);
        self.open = false;
        self.query.clear();
        self.rows.clear();
        self.requested = None;
        self.edited = None;
    }
    fn poll(&mut self) {
        if let Some(rx) = self.pending.as_ref() {
            match rx.try_recv() {
                Ok((epoch, query, rows)) => {
                    if epoch == self.epoch && epoch != u64::MAX && query == self.query {
                        self.rows = rows;
                    }
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.requested = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, selected: &mut Option<PathBuf>) -> bool {
        self.poll();
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.button("Choose game process…").clicked() {
                self.open = !self.open;
                if self.open {
                    self.refresh(ui.ctx());
                }
            }
            if selected.is_some() && ui.button("Automatic").clicked() {
                *selected = None;
                changed = true;
            }
        });
        if let Some(path) = selected.as_ref() {
            ui.label(egui::RichText::new(path.display().to_string()).small().weak()).on_hover_text("Only this executable is used as the main game process. The selection is saved with this rule.");
        } else {
            ui.label(
                egui::RichText::new("Automatic · starts tracking when the game window appears")
                    .small()
                    .weak(),
            );
        }
        if !self.open {
            return changed;
        }
        ui.label(
            "Start the game, then select its process. Steam and infrastructure processes are excluded. Choose the executable that owns the game window.",
        );
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Find game process…")
                    .char_limit(128)
                    .desired_width((ui.available_width() - 75.0).max(80.0)),
            );
            if edit.changed() {
                self.rows.clear();
                self.edited = Some(std::time::Instant::now());
            }
            if ui
                .add_enabled(self.pending.is_none(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh(ui.ctx());
            }
        });
        if self.pending.is_none() && self.requested.as_ref() != Some(&self.query) {
            let elapsed = self
                .edited
                .map(|at| at.elapsed())
                .unwrap_or(std::time::Duration::from_secs(1));
            let debounce = std::time::Duration::from_millis(250);
            if elapsed >= debounce {
                self.refresh(ui.ctx());
            } else {
                ui.ctx().request_repaint_after(debounce - elapsed);
            }
        }
        if self.pending.is_some() {
            ui.label("Reading running processes…");
        }
        let query = self.query.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("steam-game-processes")
            .max_height(160.0)
            .show(ui, |ui| {
                for row in self.rows.iter().filter(|p| {
                    p.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                }) {
                    let name = row.path.file_name().unwrap_or_default().to_string_lossy();
                    if ui
                        .selectable_label(
                            selected.as_ref() == Some(&row.path),
                            format!(
                                "{name} · PID {}{}",
                                row.pid,
                                if row.has_window { " · window" } else { "" }
                            ),
                        )
                        .on_hover_ui(|ui| {
                            ui.set_max_width(260.0);
                            ui.add(egui::Label::new(row.path.display().to_string()).wrap());
                        })
                        .clicked()
                    {
                        *selected = Some(row.path.clone());
                        self.open = false;
                        changed = true;
                    }
                }
                if self.rows.is_empty() && self.pending.is_none() {
                    ui.label("No eligible processes found. Start the game and press Refresh.");
                }
            });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s103_editor_reset_keeps_worker_ownership_and_fences_same_query_results() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut p = SteamProcessPicker {
            pending: Some(rx),
            ..Default::default()
        };
        p.reset_view();
        assert!(p.pending.is_some());
        tx.send((
            0,
            String::new(),
            vec![os_api::steam::GameProcess {
                pid: 1,
                creation_token: 1,
                path: "old.exe".into(),
                has_window: true,
            }],
        ))
        .unwrap();
        p.poll();
        assert!(p.pending.is_none());
        assert!(p.rows.is_empty());
    }
    #[test]
    fn s103_stale_picker_results_are_discarded_and_disconnected_work_can_retry() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut picker = SteamProcessPicker {
            query: "game".into(),
            pending: Some(rx),
            ..Default::default()
        };
        tx.send((
            0,
            "old".into(),
            vec![os_api::steam::GameProcess {
                pid: 1,
                creation_token: 1,
                path: "old.exe".into(),
                has_window: true,
            }],
        ))
        .unwrap();
        picker.poll();
        assert!(picker.rows.is_empty());
        assert!(picker.pending.is_none());
        let (tx, rx) = mpsc::sync_channel(1);
        picker.pending = Some(rx);
        picker.requested = Some("game".into());
        drop(tx);
        picker.poll();
        assert!(picker.pending.is_none());
        assert!(picker.requested.is_none());
    }
}
