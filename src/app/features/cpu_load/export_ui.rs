use super::{
    export::{self, ExportManager},
    history::HistoryStore,
    CpuLoadRuntime,
};
use eframe::egui;
use std::{
    io::{Seek, SeekFrom},
    sync::{mpsc, Arc, RwLock},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
enum Action {
    Png {
        rect: egui::Rect,
        cpu: usize,
    },
    #[default]
    Csv,
}
#[derive(Clone)]
struct CaptureTag(u64);
struct Capture {
    id: u64,
    rect: egui::Rect,
    scale: f32,
    viewport: egui::Vec2,
    cpu: usize,
    sent: bool,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct ExportUi {
    capture: Option<Capture>,
    result: Option<mpsc::Receiver<Result<bool, String>>>,
    pub notice: Option<String>,
}
pub(super) fn menu(
    ui: &mut egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    cpu: usize,
) -> bool {
    let capture_rect = rect.intersect(ui.clip_rect());
    let popup = response.context_menu(|ui| {
        let png = ui.button("Save this graph (PNG)…");
        crate::debug_mcp_control!("graph.export.png", png);
        if png.clicked() {
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("cpu-export-action"),
                    Action::Png {
                        rect: capture_rect,
                        cpu,
                    },
                )
            });
            ui.close();
        }
        let csv = ui.button("Save all CPU history (CSV)…");
        crate::debug_mcp_control!("graph.export.csv", csv);
        if csv.clicked() {
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("cpu-export-action"), Action::Csv));
            ui.close();
        }
    });
    if let Some(popup) = &popup {
        // Keep the owned menu above a reactivated modal of the same native order.
        if ui.layer_id().order == popup.response.layer_id.order {
            ui.ctx()
                .set_sublayer(ui.layer_id(), popup.response.layer_id);
        }
    }
    popup.is_some()
}
fn save_csv(
    manager: Arc<ExportManager>,
    history: Arc<RwLock<HistoryStore>>,
) -> Result<bool, String> {
    let artifact = manager.capture(&history)?;
    let Some(path) = rfd::FileDialog::new()
        .add_filter("CSV spreadsheet", &["csv"])
        .set_file_name(&artifact.name)
        .save_file()
    else {
        return Ok(false);
    };
    let mut input = artifact.file.lock().map_err(|_| "Export is busy")?;
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut output = std::fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::copy(&mut *input, &mut output).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(true)
}
fn crop(
    image: &egui::ColorImage,
    rect: egui::Rect,
    scale: f32,
) -> Result<(Vec<u8>, u32, u32), String> {
    if !rect.is_finite()
        || scale <= 0.0
        || !scale.is_finite()
        || image.size[0].saturating_mul(image.size[1]) > 16 * 1024 * 1024
    {
        return Err("Screenshot size is unsupported.".into());
    }
    let x0 = (rect.left() * scale).floor().max(0.0) as usize;
    let y0 = (rect.top() * scale).floor().max(0.0) as usize;
    let x1 = (rect.right() * scale).ceil().min(image.size[0] as f32) as usize;
    let y1 = (rect.bottom() * scale).ceil().min(image.size[1] as f32) as usize;
    if x0 >= x1 || y0 >= y1 {
        return Err("Graph is outside the visible window. Show it and try again.".into());
    }
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        for pixel in &image.pixels[y * image.size[0] + x0..y * image.size[0] + x1] {
            pixels.extend_from_slice(&pixel.to_array());
        }
    }
    Ok((pixels, (x1 - x0) as u32, (y1 - y0) as u32))
}
impl ExportUi {
    fn worker(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<bool, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::sync_channel(1);
        self.result = Some(rx);
        self.notice = Some("Preparing export…".into());
        let wake = ctx.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("cpu-export-save".into())
            .spawn(move || {
                let _ = tx.send(work());
                wake.request_repaint();
            })
        {
            self.result = None;
            self.notice = Some(e.to_string());
        }
    }
    fn poll(&mut self) {
        if let Some(result) = self.result.as_ref().and_then(|r| r.try_recv().ok()) {
            self.result = None;
            self.notice = Some(match result {
                Ok(true) => "Export saved.".into(),
                Ok(false) => "Export cancelled.".into(),
                Err(e) => format!("Export failed: {e}"),
            });
        }
    }
}
impl CpuLoadRuntime {
    pub(crate) fn export_input(&mut self, ctx: &egui::Context, input: &egui::RawInput) {
        self.export_ui.poll();
        if self
            .export_ui
            .capture
            .as_ref()
            .is_some_and(|c| c.deadline < Instant::now())
        {
            self.export_ui.capture = None;
            self.export_ui.notice =
                Some("Screenshot timed out. Restore the window and try again.".into());
        }
        let screenshot = input.events.iter().find_map(|e| match e {
            egui::Event::Screenshot {
                user_data, image, ..
            } if user_data
                .data
                .as_ref()
                .and_then(|d| d.downcast_ref::<CaptureTag>())
                .is_some_and(|tag| {
                    self.export_ui
                        .capture
                        .as_ref()
                        .is_some_and(|c| c.id == tag.0)
                }) =>
            {
                Some(image.clone())
            }
            _ => None,
        });
        if let Some(image) = screenshot {
            if let Some(capture) = self.export_ui.capture.take() {
                if (capture.scale - ctx.pixels_per_point()).abs() > 0.001 {
                    self.export_ui.notice =
                        Some("Window scale changed. Save the graph again.".into());
                    return;
                }
                if ((image.size[0] as f32 - capture.viewport.x * capture.scale).abs() > 2.0)
                    || ((image.size[1] as f32 - capture.viewport.y * capture.scale).abs() > 2.0)
                {
                    self.export_ui.notice =
                        Some("Window size changed. Save the graph again.".into());
                    return;
                }
                self.export_ui.worker(ctx, move || {
                    let (pixels, width, height) = crop(&image, capture.rect, capture.scale)?;
                    let name = export::filename(&format!("cpu-{}-graph", capture.cpu), "png");
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG image", &["png"])
                        .set_file_name(name)
                        .save_file()
                    else {
                        return Ok(false);
                    };
                    image::save_buffer_with_format(
                        path,
                        &pixels,
                        width,
                        height,
                        image::ColorType::Rgba8,
                        image::ImageFormat::Png,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(true)
                });
            }
        }
        if let Some(capture) = self.export_ui.capture.as_mut().filter(|c| !c.sent) {
            capture.sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                CaptureTag(capture.id),
            )));
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }
    pub(super) fn export_actions(&mut self, ctx: &egui::Context) {
        let action = ctx.data_mut(|d| d.remove_temp::<Action>(egui::Id::new("cpu-export-action")));
        let Some(action) = action else { return };
        if self.export_ui.capture.is_some() || self.export_ui.result.is_some() {
            self.export_ui.notice = Some("An export is already in progress.".into());
            return;
        }
        match action {
            Action::Png { rect, cpu } => {
                self.export_ui.capture = Some(Capture {
                    id: ctx.cumulative_frame_nr(),
                    rect,
                    cpu,
                    scale: ctx.pixels_per_point(),
                    viewport: ctx.content_rect().size(),
                    sent: false,
                    deadline: Instant::now() + Duration::from_secs(5),
                });
                ctx.request_repaint();
            }
            Action::Csv => {
                let manager = self.exports.clone();
                let history = self.history.clone();
                self.export_ui
                    .worker(ctx, move || save_csv(manager, history));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_menu_stays_above_its_reactivated_modal_parent() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("export-modal-parent");
        let parent = egui::LayerId::new(egui::Order::Foreground, id);
        let pos = egui::pos2(25.0, 15.0);
        let mut output = egui::FullOutput::default();
        for step in 0..9 {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if step == 3 || step == 4 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: step == 3,
                    modifiers: Default::default(),
                });
            }
            output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |_| {
                    egui::Area::new(id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::Pos2::ZERO)
                        .show(&ctx, |ui| {
                            ui.set_min_size(egui::vec2(350.0, 220.0));
                            let chart = ui.add_sized([120.0, 30.0], egui::Button::new("Chart"));
                            menu(ui, &chart, chart.rect, 0);
                        });
                    ctx.move_to_top(parent);
                },
            );
            output.textures_delta.clear();
        }
        let point = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Save this graph (PNG)…" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Menu must open");
        assert_ne!(
            ctx.layer_id_at(point),
            Some(parent),
            "Reactivating the modal must not cover its owned menu"
        );
    }
    #[test]
    fn png_crop_uses_physical_scale_and_excludes_the_rest_of_the_window() {
        let mut image = egui::ColorImage::filled([8, 8], egui::Color32::RED);
        for y in 2..6 {
            for x in 2..6 {
                image.pixels[y * 8 + x] = egui::Color32::GREEN;
            }
        }
        let (data, w, h) = crop(
            &image,
            egui::Rect::from_min_max(egui::pos2(1.0, 1.0), egui::pos2(3.0, 3.0)),
            2.0,
        )
        .unwrap();
        assert_eq!((w, h), (4, 4));
        assert!(data.chunks_exact(4).all(|p| p == [0, 255, 0, 255]));
        use image::ImageEncoder;
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&data, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            image::load_from_memory(&encoded)
                .unwrap()
                .into_rgba8()
                .as_raw(),
            &data
        );
    }
}
