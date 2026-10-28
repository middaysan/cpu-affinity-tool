use super::runtime::{ConnectionCopy, McpRuntime};
use crate::app::shell::presenters::shared_elements::{
    modal, palette, service_control, service_modal_header, service_modal_layout, service_tabs,
    toned_button, ToneRole,
};
use eframe::egui;

impl McpRuntime {
    pub(crate) fn draw(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if self.logs_selected {
            self.refresh_activity(true);
        }
        let mut candidate = self.policy.clone();
        candidate.port = self.port_draft.unwrap_or(candidate.port);
        let mut commit = false;
        let mut copy = None;
        let ready = self.connection_ready();
        let response = modal(egui::Id::new("production-mcp-connection")).show(ctx, |ui| {
            let body_height = if !self.logs_selected {140.0} else if !self.activity_rows.is_empty() {260.0} else {110.0};
            service_modal_layout(ui, body_height);
            let close = service_modal_header(ui, "MCP server", "Local AI access to apps, groups and System Tweaks.");
            crate::debug_mcp_control!("mcp.connection.close", close);
            if close.clicked() { self.open = false; }
            let colors = *palette(ui);
            let (status, color) = if self.busy() {
                ("Updating…", colors.text_muted)
            } else if self.listening() {
                ("Listening", colors.success.fg)
            } else if self.enabled() {
                ("Starting…", colors.warning.fg)
            } else { ("Off", colors.text_muted) };
            let control = service_control(ui, &mut candidate.enabled, ready, "Enable MCP server", status, color).on_hover_text(format!("Local connection · Port {}", self.endpoint_port()));
            crate::debug_mcp_control!("settings.production_mcp", control);
            commit |= control.changed();
            let [connection, logs] = service_tabs(ui, &mut self.logs_selected, ["Connection", "Logs"]);
            crate::debug_mcp_control!("mcp.tab.connection", connection);
            crate::debug_mcp_control!("mcp.tab.logs", logs);
            commit |= connection.changed() || logs.changed();
            if logs.changed() && self.logs_selected { self.refresh_activity(true); }
            if let Some(error) = &self.error {
                ui.add(egui::Label::new(egui::RichText::new(error).color(ui.visuals().error_fg_color)).wrap());
            }
            if self.logs_selected {
                self.draw_activity_table(ui);
            } else {
                egui::ScrollArea::vertical().id_salt("mcp-connection-body")
                    .max_height(ui.available_height().max(40.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Port");
                            let port = ui.add_enabled(ready, egui::DragValue::new(&mut candidate.port).range(1024..=65535).speed(1));
                            crate::debug_mcp_control!("mcp.port", port);
                            if port.changed() { self.port_draft = Some(candidate.port); }
                            commit |= port.drag_stopped() || port.lost_focus();
                        });
                        ui.add_enabled_ui(ready, |ui| ui.horizontal(|ui| {
                            let width=(ui.available_width()-ui.spacing().item_spacing.x)/2.0;
                            let connection=ui.add(egui::Button::new("Copy connection").min_size(egui::vec2(width,21.0)))
                                .on_hover_ui(|ui| setup_hint(ui,"Copy the URL and Authorization header for your agent's Streamable HTTP MCP settings."));
                            crate::debug_mcp_control!("mcp.copy_connection",connection);
                            if connection.clicked() {copy=Some(ConnectionCopy::Json);}
                            let prompt=toned_button(ui,egui::Button::new("Copy AI prompt").min_size(egui::vec2(width,21.0)),ToneRole::Primary)
                                .on_hover_ui(|ui| setup_hint(ui,"Copy a complete setup prompt, including the address and private key. Paste it into a fresh session with your trusted local AI agent."));
                            crate::debug_mcp_control!("mcp.copy_ai_prompt",prompt);
                            if prompt.clicked() {copy=Some(ConnectionCopy::AiPrompt);}
                        }));
                        ui.add(egui::Label::new(egui::RichText::new("Use AI prompt for guided setup, or connection data for manual setup. This PC only.").size(9.5).color(colors.text_secondary)).wrap());
                        ui.add(egui::Label::new(egui::RichText::new("Full-access key. Use only with an agent you trust.").size(9.5).color(colors.text_muted)).wrap())
                            .on_hover_ui(|ui| setup_hint(ui,"The permanent key gives full access, including launching programs and changing settings. Share it only with an agent you trust."));
                    });
            }
        });
        if response.should_close() {
            self.open = false;
        }
        let dirty = candidate.port != self.policy.port || candidate.enabled != self.policy.enabled;
        if dirty && (commit || copy.is_some() || !self.open) && ready {
            self.save(candidate, ctx);
            self.copy_after_save = copy.filter(|_| self.busy());
        } else if let Some(format) = copy {
            self.copy_connection(ctx, format);
        }
        if !self.open {
            self.port_draft = None;
            self.activity_rows.clear();
            self.activity_revision = 0;
        }
    }
}

fn setup_hint(ui: &mut egui::Ui, text: &str) {
    ui.set_width((ui.ctx().content_rect().width() - 32.0).clamp(120.0, 260.0));
    ui.add(egui::Label::new(text).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dark: bool) -> (McpRuntime, egui::Context) {
        let mut mcp = McpRuntime::default();
        mcp.open = true;
        mcp.policy.token = "a".repeat(64);
        let ctx = egui::Context::default();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
        ctx.global_style_mut(|style| {
            crate::app::shell::presenters::shared_elements::apply_widget_style(style);
            style.animation_time = 0.0;
        });
        (mcp, ctx)
    }
    fn frame(
        mcp: &mut McpRuntime,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| mcp.draw(ui.ctx()),
        );
        output.textures_delta.clear();
        output
    }
    fn settle(mcp: &mut McpRuntime, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let mut output = frame(mcp, ctx, vec![], size);
        for _ in 0..3 {
            output = frame(mcp, ctx, vec![], size);
        }
        output
    }
    fn label(output: &egui::FullOutput, label: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    if text.galley.job.text == label && shape.clip_rect.contains_rect(rect) {
                        return Some(rect);
                    }
                }
                None
            })
            .unwrap_or_else(|| panic!("Missing visible label: {label}"))
    }
    fn click(mcp: &mut McpRuntime, ctx: &egui::Context, point: egui::Pos2) -> egui::FullOutput {
        let size = egui::vec2(400.0, 600.0);
        frame(
            mcp,
            ctx,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            size,
        );
        frame(
            mcp,
            ctx,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            size,
        )
    }
    fn clipboard(output: &egui::FullOutput) -> Option<&str> {
        output.platform_output.commands.iter().find_map(|cmd| {
            if let egui::OutputCommand::CopyText(text) = cmd {
                Some(text.as_str())
            } else {
                None
            }
        })
    }
    #[test]
    fn connection_copy_actions_include_a_self_contained_agent_prompt() {
        for dark in [true, false] {
            let (mut mcp, ctx) = fixture(dark);
            mcp.policy.port = 40123;
            let output = settle(&mut mcp, &ctx, egui::vec2(400.0, 600.0));
            let json = label(&output, "Copy connection");
            let prompt = label(&output, "Copy AI prompt");
            assert!((json.center().y - prompt.center().y).abs() < 1.0);
            assert!(json.right() < prompt.left());
            assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text.contains(&mcp.policy.token))));
            let output = click(&mut mcp, &ctx, prompt.center());
            let text = clipboard(&output).expect("copied complete AI prompt");
            let connection: serde_json::Value = serde_json::from_str(
                text.split("```json\n")
                    .nth(1)
                    .unwrap()
                    .split("\n```")
                    .next()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(connection["url"], "http://127.0.0.1:40123/mcp");
            assert_eq!(
                connection["headers"]["Authorization"],
                format!("Bearer {}", mcp.policy.token)
            );
            assert_eq!(text.matches(&mcp.policy.token).count(), 1);
            for required in [
                "Streamable HTTP",
                "get_capabilities",
                "tools/list",
                "get_app_state",
                "request_key",
                "expected_revision",
                "get_operation",
                "read-only",
                "restart",
                "untrusted data",
                "update_rule is a complete replacement",
                "additional_processes",
                "bin_path",
            ] {
                assert!(text.contains(required), "Prompt lacks {required}");
            }
        }
    }
    #[test]
    fn connection_copy_tooltip_wraps_within_a_small_viewport() {
        for dark in [true, false] {
            let (mut mcp, ctx) = fixture(dark);
            ctx.global_style_mut(|s| {
                s.interaction.tooltip_delay = 0.0;
                s.interaction.show_tooltips_only_when_still = false;
                s.wrap_mode = Some(egui::TextWrapMode::Extend);
            });
            let size = egui::vec2(360.0, 500.0);
            let output = settle(&mut mcp, &ctx, size);
            let point = label(&output, "Copy connection").center();
            let mut output = frame(&mut mcp, &ctx, vec![egui::Event::PointerMoved(point)], size);
            for _ in 0..5 {
                output = frame(&mut mcp, &ctx, vec![], size);
            }
            let tip="Copy the URL and Authorization header for your agent's Streamable HTTP MCP settings.";
            let rect = label(&output, tip);
            assert!(
                rect.width() <= 260.5 && rect.height() > 20.0,
                "Tooltip must wrap: {rect:?}"
            );
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
        }
    }
    #[test]
    fn service_layout_has_clear_hierarchy_and_compact_empty_logs() {
        for dark in [true, false] {
            let (mut mcp, ctx) = fixture(dark);
            for size in [egui::vec2(400.0, 1000.0), egui::vec2(360.0, 400.0)] {
                mcp.logs_selected = false;
                let output = settle(&mut mcp, &ctx, size);
                let title = label(&output, "MCP server");
                let purpose = label(
                    &output,
                    "Local AI access to apps, groups and System Tweaks.",
                );
                let toggle = label(&output, "Enable MCP server");
                assert!(title.height() > toggle.height());
                assert!(title.bottom() < purpose.top() && purpose.bottom() < toggle.top());
                let connection = label(&output, "Connection");
                let logs = label(&output, "Logs");
                assert!((connection.center().y - logs.center().y).abs() < 1.0);
                assert!(logs.center().x - connection.center().x > 120.0);
                for name in ["Port", "Copy connection", "Copy AI prompt"] {
                    label(&output, name);
                }
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        assert!(![
                            "AI clients",
                            "Permissions",
                            "Choose apps",
                            "Additional permissions",
                            "Selected client",
                            "Save changes"
                        ]
                        .iter()
                        .any(|name| text.galley.job.text.contains(name)));
                    }
                }
                assert!(label(&output, "Copy connection").bottom() - title.top() < 180.0);
                mcp.logs_selected = true;
                let output = settle(&mut mcp, &ctx, size);
                label(&output, "No commands yet");
                assert!(!output.shapes.iter().any(
                    |s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="Command")
                ));
                let rect = ctx
                    .memory(|m| m.area_rect(egui::Id::new("production-mcp-connection")))
                    .unwrap();
                assert!(
                    rect.height() <= 290.0,
                    "Empty logs should be compact: {rect:?}"
                );
                assert!(rect.top() >= 0.0 && rect.bottom() <= size.y);
            }
        }
    }
    #[test]
    fn service_log_errors_stay_visible_above_the_scroll_list() {
        use super::super::{activity::CommandStatus, operations::Ledger};
        use std::sync::{Arc, Mutex};
        let (mut mcp, ctx) = fixture(true);
        let mut ledger = Ledger::new("epoch".into());
        for _ in 0..30 {
            let id = ledger
                .commands
                .begin("get_app_state", std::time::Instant::now())
                .0;
            ledger.commands.finish(id, CommandStatus::Done, None);
        }
        mcp.ledger = Some(Arc::new(Mutex::new(ledger)));
        mcp.logs_selected = true;
        let error="The connection could not be saved. Check access to the protected settings directory before trying again.";
        mcp.error = Some(error.into());
        let size = egui::vec2(360.0, 440.0);
        let output = settle(&mut mcp, &ctx, size);
        let error_rect = label(&output, error);
        let headings = label(&output, "Command");
        assert!(
            error_rect.bottom() < headings.top(),
            "Service errors must precede the scrolling list"
        );
        let modal = ctx
            .memory(|m| m.area_rect(egui::Id::new("production-mcp-connection")))
            .unwrap();
        assert!(modal.bottom() <= size.y && modal.top() >= 0.0, "{modal:?}");
    }
    #[test]
    fn switching_from_connection_to_logs_keeps_a_useful_visible_list() {
        use super::super::{activity::CommandStatus, operations::Ledger};
        use std::sync::{Arc, Mutex};
        let (mut mcp, ctx) = fixture(true);
        let size = egui::vec2(400.0, 700.0);
        settle(&mut mcp, &ctx, size);
        let mut ledger = Ledger::new("epoch".into());
        for _ in 0..100 {
            let id = ledger
                .commands
                .begin("get_app_state", std::time::Instant::now())
                .0;
            ledger.commands.finish(id, CommandStatus::Done, None);
        }
        mcp.ledger = Some(Arc::new(Mutex::new(ledger)));
        mcp.logs_selected = true;
        let output = settle(&mut mcp, &ctx, size);
        let visible=output.shapes.iter().filter(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="Done" && shape.clip_rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())))).count();
        assert!(
            visible >= 12,
            "The journal viewport was clipped to {visible} visible rows"
        );
    }
    #[test]
    fn off_header_button_opens_connection_controls_and_logs_remain_reachable() {
        let (mut mcp, ctx) = fixture(true);
        mcp.open = false;
        let size = egui::vec2(400.0, 600.0);
        let mut point = egui::Pos2::ZERO;
        for step in 0..6 {
            let events = if step >= 4 {
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: step == 4,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ui| {
                    mcp.status_button(ui, 120.0);
                    mcp.draw(ui.ctx());
                },
            );
            output.textures_delta.clear();
            if step < 4 {
                point = label(&output, "MCP").center();
            }
        }
        assert!(mcp.open && !mcp.enabled());
        let output = settle(&mut mcp, &ctx, size);
        for name in ["Enable MCP server", "Connection", "Logs", "Copy connection"] {
            label(&output, name);
        }
        mcp.logs_selected = true;
        let output = settle(&mut mcp, &ctx, size);
        for name in ["Enable MCP server", "No commands yet"] {
            label(&output, name);
        }
    }
    #[test]
    fn connection_dialog_uses_shared_scrim_and_escape() {
        for dark in [true, false] {
            let (mut mcp, ctx) = fixture(dark);
            let size = egui::vec2(400.0, 600.0);
            let output = settle(&mut mcp, &ctx, size);
            assert_eq!(
                ctx.memory(|m| m.top_modal_layer()).map(|layer| layer.id),
                Some(egui::Id::new("production-mcp-connection"))
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill==egui::Color32::from_black_alpha(185) && rect.rect==egui::Rect::from_min_size(egui::Pos2::ZERO,size))));
            frame(
                &mut mcp,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                size,
            );
            assert!(!mcp.open);
        }
    }
    #[test]
    fn copy_uses_the_persisted_key_and_port_without_exposing_it_in_labels() {
        let (mut mcp, ctx) = fixture(true);
        mcp.policy.port = 40123;
        let output = settle(&mut mcp, &ctx, egui::vec2(400.0, 600.0));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.contains(&mcp.policy.token))));
        let output = click(&mut mcp, &ctx, label(&output, "Copy connection").center());
        let connection: serde_json::Value =
            serde_json::from_str(clipboard(&output).expect("copied connection")).unwrap();
        assert_eq!(connection["url"], "http://127.0.0.1:40123/mcp");
        assert_eq!(
            connection["headers"]["Authorization"],
            format!("Bearer {}", mcp.policy.token)
        );
    }
    #[test]
    fn copy_never_publishes_a_pending_key_or_unsaved_port() {
        for button in ["Copy connection", "Copy AI prompt"] {
            let (mut mcp, ctx) = fixture(true);
            mcp.policy.token.clear();
            let output = settle(&mut mcp, &ctx, egui::vec2(400.0, 600.0));
            let output = click(&mut mcp, &ctx, label(&output, button).center());
            assert!(clipboard(&output).is_none());
            mcp.policy.token = "a".repeat(64);
            mcp.port_draft = Some(40099);
            let output = settle(&mut mcp, &ctx, egui::vec2(400.0, 600.0));
            let output = click(&mut mcp, &ctx, label(&output, button).center());
            assert!(clipboard(&output).is_none());
            assert_eq!(mcp.policy.port, super::super::policy::DEFAULT_PORT);
            assert!(mcp.error.is_some());
            assert!(mcp.copy_after_save.is_none());
        }
    }
}
