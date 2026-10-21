use super::{
    activity::{command_label, flash_remaining, CommandStatus},
    runtime::McpRuntime,
};
use crate::app::shell::presenters::{
    process_table as table,
    shared_elements::{
        compact_table_spacing, palette, service_empty_state, table_row_separator, wrapped_tooltip,
        COMPACT_TABLE_ROW_HEIGHT,
    },
};
use eframe::egui;
use std::time::Instant;
impl McpRuntime {
    pub(super) fn refresh_activity(&mut self, rows: bool) {
        if let Some(ledger) = self
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.try_lock().ok())
        {
            self.last_activity = ledger.commands.last_received;
            if rows && self.activity_revision != ledger.commands.revision {
                self.activity_rows = ledger.commands.rows.iter().rev().cloned().collect();
                self.activity_revision = ledger.commands.revision;
            }
        }
    }
    pub(crate) fn status_button(&mut self, ui: &mut egui::Ui, width: f32) {
        self.refresh_activity(false);
        let connected = self.listening();
        let remaining = connected
            .then(|| flash_remaining(self.last_activity, Instant::now()))
            .flatten();
        let colors = *palette(ui);
        let color = if remaining.is_some() {
            colors.danger.fg
        } else if connected {
            colors.success.fg
        } else {
            colors.neutral_status
        };
        let response = crate::app::shell::presenters::shared_elements::status_button(
            ui,
            [width, 21.0],
            "MCP",
            color,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                "MCP server. Open local AI connection controls and command logs.",
            )
        });
        crate::debug_mcp_control!("mcp.status", response);
        let response = wrapped_tooltip(
            response,
            if connected {
                "Local AI access is running. Click to see the last 100 commands."
            } else if self.enabled() {
                "MCP is starting. Click to see its status and command log."
            } else {
                "Local AI access is off. Click for connection settings and logs."
            },
        );
        if response.clicked() {
            self.open = true;
        }
        if let Some(remaining) = remaining {
            ui.ctx().request_repaint_after(remaining);
        }
    }
    pub(super) fn draw_activity_table(&mut self, ui: &mut egui::Ui) {
        let empty = self.activity_rows.is_empty();
        let rect = if empty {
            service_empty_state(
                ui,
                "No commands yet",
                "Commands from your AI agent will appear here.",
            )
            .rect
        } else {
            ui.label(
                egui::RichText::new("Last 100 commands · newest first · this app session")
                    .size(9.0)
                    .color(palette(ui).text_muted),
            );
            compact_table_spacing(ui);
            ui.spacing_mut().item_spacing.x = 6.0;
            let content_width =
                (ui.available_width() - ui.spacing().scroll.allocated_width() - 4.0).max(150.0);
            let widths = [46.0, (content_width - 46.0 - 64.0 - 12.0).max(30.0), 64.0];
            ui.horizontal(|ui| {
                for (i, title) in ["Time", "Command", "Result"].into_iter().enumerate() {
                    table::cell(
                        ui,
                        widths[i],
                        i == 0,
                        egui::RichText::new(title).size(9.0).weak(),
                    );
                }
            });
            ui.separator();
            let height = ui.available_height().max(30.0);
            let scroll=egui::ScrollArea::vertical().id_salt("mcp-command-list").max_height(height).min_scrolled_height(height).auto_shrink([false,false]).content_margin(0)
                .show_rows(ui,COMPACT_TABLE_ROW_HEIGHT,self.activity_rows.len(),|ui,range| {
                    for index in range {
                        let row=&self.activity_rows[index];
                        let r=ui.push_id(row.sequence,|ui|ui.horizontal(|ui| {
                            table::cell(ui,widths[0],true,egui::RichText::new(os_api::cpu_load::sample_clock_label(row.time)).size(9.0));
                            table::cell(ui,widths[1],false,egui::RichText::new(command_label(row.tool)).size(9.0)).on_hover_text(format!("{}\nMCP: {}",command_label(row.tool),row.tool));
                            let colors=palette(ui);
                            let color=match row.status {CommandStatus::Done=>colors.success.fg,CommandStatus::Failed(_)=>colors.danger.fg,CommandStatus::Unknown=>colors.warning.fg,_=>colors.text_muted};
                            let detail=match row.status {CommandStatus::Failed("validation")=>"The command contained invalid settings or arguments.",CommandStatus::Failed(_)=>"The command failed. See the application's logs or the operation receipt for details.",CommandStatus::Unknown=>"The final result is uncertain. The agent should inspect the target before retrying.",CommandStatus::Queued=>"Accepted; waiting for execution. This is not a completed change.",CommandStatus::Running=>"The command is being processed.",CommandStatus::Done=>"Completed successfully."};
                            table::cell(ui,widths[2],false,egui::RichText::new(row.status.label()).size(9.0).color(color)).on_hover_text(detail);
                        }));
                        table_row_separator(ui,r.inner.response.rect);
                    }
                });
            scroll.inner_rect
        };
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(
            ui.ctx(),
            "mcp.activity.table",
            rect,
            true,
            serde_json::json!({"rows":self.activity_rows.len(),"revision":self.activity_revision,"newest":self.activity_rows.first().map(|r|r.sequence),"oldest":self.activity_rows.last().map(|r|r.sequence),"newest_status":self.activity_rows.first().map(|r|r.status.label())}),
        );
        #[cfg(not(all(debug_assertions, feature = "debug-mcp")))]
        let _ = rect;
    }
}
