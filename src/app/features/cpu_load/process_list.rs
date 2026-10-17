use super::model::RuleTarget;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

pub(super) struct ProcessRow {
    pub target: RuleTarget,
    pub pid: u32,
    pub token: u64,
    identity: Option<Arc<os_api::cpu_load::CpuProcess>>,
    /// Whole-machine CPU time, independent of scheduler attribution.
    pub total: Option<f32>,
    pub cores: Vec<(usize, f32)>,
    pub known_cores: usize,
    pub cpu_count: usize,
    pid_text: String,
    load_text: String,
    cores_text: String,
}
impl ProcessRow {
    pub fn pid_text(&self) -> &str {
        &self.pid_text
    }
    pub fn load_text(&self) -> &str {
        &self.load_text
    }
    pub fn cores_text(&self) -> &str {
        &self.cores_text
    }
    fn update_cores_text(&mut self) {
        use std::fmt::Write;
        self.cores_text.clear();
        let complete = self.cpu_count > 0 && self.known_cores == self.cpu_count;
        if self.known_cores == 0 {
            self.cores_text.push_str("Unavailable");
        } else if self.cores.is_empty() {
            self.cores_text
                .push_str(if complete { "Idle" } else { "Partial" });
        } else {
            for (index, (cpu, _)) in self.cores.iter().enumerate() {
                if index > 0 {
                    self.cores_text.push_str(", ");
                }
                let _ = write!(self.cores_text, "{cpu}");
            }
            if !complete {
                self.cores_text.push_str(" *");
            }
        }
    }
    pub fn name(&self) -> &str {
        self.identity
            .as_ref()
            .map(|p| p.name.as_ref())
            .unwrap_or("Name unavailable")
    }
}

#[derive(Default)]
pub(super) struct ProcessList {
    pub rows: Vec<ProcessRow>,
    pub ready: bool,
    pub time: Option<SystemTime>,
}
impl ProcessList {
    pub fn from_tracking(
        targets: &HashMap<RuleTarget, Vec<(u32, u64)>>,
        totals: &HashMap<(u32, u64), Option<f32>>,
        names: &HashMap<(u32, u64), Arc<os_api::cpu_load::CpuProcess>>,
    ) -> Self {
        let mut rows: Vec<_> = targets
            .iter()
            .flat_map(|(target, instances)| {
                instances.iter().map(move |&(pid, token)| ProcessRow {
                    target: target.clone(),
                    pid,
                    token,
                    identity: names
                        .get(&(pid, token))
                        .filter(|name| name.pid == pid && name.instance == token)
                        .cloned(),
                    total: totals.get(&(pid, token)).copied().flatten(),
                    cores: Vec::new(),
                    known_cores: 0,
                    cpu_count: 0,
                    pid_text: pid.to_string(),
                    load_text: super::view::percent(totals.get(&(pid, token)).copied().flatten()),
                    cores_text: "Unavailable".into(),
                })
            })
            .collect();
        rows.sort_unstable_by(|a, b| {
            a.pid
                .cmp(&b.pid)
                .then(a.token.cmp(&b.token))
                .then(a.target.group.0.cmp(&b.target.group.0))
                .then(a.target.rule.0.cmp(&b.target.rule.0))
        });
        rows.dedup_by(|a, b| a.pid == b.pid && a.token == b.token && a.target == b.target);
        Self {
            rows,
            ready: true,
            time: None,
        }
    }
    pub fn record_cores(&mut self, count: usize, samples: &[os_api::cpu_load::ProcessCpuSample]) {
        for row in &mut self.rows {
            row.cores.clear();
            row.known_cores = 0;
            row.cpu_count = count;
            let sample = samples
                .binary_search_by_key(&(row.pid, row.token), |s| (s.pid, s.token))
                .ok()
                .and_then(|index| samples.get(index));
            if let Some(sample) = sample.filter(|sample| sample.cpus.len() == count) {
                for (cpu, value) in sample.cpus.iter().enumerate() {
                    if let Some(value) = value.filter(|v| v.is_finite()) {
                        row.known_cores += 1;
                        if value > 0.0 {
                            row.cores.push((cpu, value));
                        }
                    }
                }
            }
            row.update_cores_text();
        }
    }
    pub fn invalidate(&mut self) {
        self.ready = false;
        for row in &mut self.rows {
            row.total = None;
            row.known_cores = 0;
            row.cores.clear();
            row.load_text = super::view::percent(None);
            row.update_cores_text();
        }
    }
    pub fn reserved_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<ProcessRow>()
            + self
                .rows
                .iter()
                .map(|row| {
                    row.cores.capacity() * std::mem::size_of::<(usize, f32)>()
                        + row.target.group.0.capacity()
                        + row.target.rule.0.capacity()
                        + row.pid_text.capacity()
                        + row.load_text.capacity()
                        + row.cores_text.capacity()
                })
                .sum::<usize>()
    }
}

pub(super) fn draw(
    ui: &mut eframe::egui::Ui,
    list: &ProcessList,
    target: &RuleTarget,
    available_height: f32,
) {
    crate::render_profile_scope!(CurrentProcesses);
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    super::debug_api::record_processes(ui.ctx(), list, target);
    use crate::app::shell::presenters::shared_elements::palette;
    use eframe::egui::{self, Align, Layout, RichText};
    let colors = *palette(ui);
    let top = ui.cursor().top();
    let rows = || list.rows.iter().filter(|row| &row.target == target);
    let count = rows().count();
    let mapped = rows()
        .filter(|r| r.cpu_count > 0 && r.known_cores == r.cpu_count)
        .count();
    let clock = list
        .time
        .map(os_api::cpu_load::sample_clock_label)
        .unwrap_or_else(|| "—".into());
    ui.label(
        RichText::new(format!("{count} tracked processes · Live · {clock}"))
            .small()
            .strong(),
    );
    ui.label(RichText::new(if list.ready {format!("CPU mapping: {mapped}/{count} processes")} else {"Updating tracked processes…".into()}).small().color(if !list.ready||mapped<count {colors.warning.fg}else{colors.text_secondary}))
        .on_hover_text("The footer and this table use tracked exact PID/creation-token instances. Refresh times can differ. This current table includes idle and unavailable instances; graph contributors include only measured work on the selected CPU/time. An unknown sibling can make the app's complete aggregate unavailable.");
    let widths = [(ui.available_width() - 202.0).max(46.0), 46.0, 46.0, 86.0];
    let cell = |ui: &mut egui::Ui, text: RichText, width: f32, right: bool| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(if right {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        }));
        child.add(egui::Label::new(text).truncate().selectable(false))
    };
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x=6.0;
        ui.horizontal(|ui|for (i,title) in ["Process","PID","CPU %","CPUs"].iter().enumerate(){
            let response=cell(ui,RichText::new(*title).small().weak(),widths[i],i==1||i==2);
            if i==2 {response.on_hover_text("Process CPU time relative to the whole machine, sampled once per second with GetProcessTimes. This can remain available when scheduler/per-core attribution is unavailable. Its interval is not identical to Task Manager or the per-CPU trace.");}
            else if i==3 {response.on_hover_text("Logical CPUs with measured positive work in the latest scheduler interval. A process can run on several CPUs; this is not a single instantaneous location or its allowed affinity. Idle means a complete measured zero interval; Unavailable and * preserve missing/partial data.");}
        });
        ui.separator();
        let height=(available_height-(ui.cursor().top()-top)).max(60.0);
        let scroll = egui::ScrollArea::vertical().id_salt("cpu-process-list").min_scrolled_height(height).max_height(height).auto_shrink([false,false]).show_rows(ui,18.0,count,|ui,range| {
            if count==0 {ui.label(if list.ready{"No tracked processes for this app."}else{"Waiting for the first tracked-process snapshot…"});}
            for row in rows().skip(range.start).take(range.len()) {
                ui.push_id((row.pid,row.token),|ui|ui.horizontal(|ui| {
                    cell(ui,RichText::new(row.name()).small(),widths[0],false).on_hover_text(row.name());
                    cell(ui,RichText::new(row.pid_text()).small(),widths[1],true);
                    cell(ui,RichText::new(row.load_text()).small(),widths[2],true);
                    let complete=row.cpu_count>0&&row.known_cores==row.cpu_count;
                    cell(ui,RichText::new(row.cores_text()).small().color(if complete{colors.text_primary}else{colors.warning.fg}),widths[3],false).on_hover_ui(|ui| {
                        ui.label(format!("CPU mapping: {}/{} logical CPUs measured",row.known_cores,row.cpu_count));
                        for &(cpu,load) in &row.cores {ui.label(format!("CPU{cpu}: {}",super::view::percent(Some(load))));}
                    });
                }));
            }
        });
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(ui.ctx(), "table.processes", scroll.inner_rect.intersect(ui.clip_rect()), true, serde_json::json!({"offset":scroll.state.offset.y,"rows":count,"content_height":scroll.content_size.y}));
        #[cfg(not(all(debug_assertions, feature = "debug-mcp")))]
        let _ = scroll;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shared::ids::{GroupId, RuleId};

    #[test]
    fn cached_row_text_tracks_partial_complete_and_invalidated_snapshots() {
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let mut list = ProcessList::from_tracking(
            &HashMap::from([(target, vec![(42, 100)])]),
            &HashMap::from([((42, 100), Some(0.5))]),
            &HashMap::new(),
        );
        assert_eq!(list.rows[0].pid_text(), "42");
        assert_eq!(list.rows[0].load_text(), "<1%");
        assert_eq!(list.rows[0].cores_text(), "Unavailable");
        list.record_cores(
            2,
            &[os_api::cpu_load::ProcessCpuSample {
                pid: 42,
                token: 100,
                cpus: vec![Some(3.0), None],
            }],
        );
        assert_eq!(list.rows[0].cores_text(), "0 *");
        let label_ptr = list.rows[0].cores_text().as_ptr();
        for _ in 0..100 {
            assert_eq!(list.rows[0].cores_text().as_ptr(), label_ptr);
        }
        list.record_cores(
            2,
            &[os_api::cpu_load::ProcessCpuSample {
                pid: 42,
                token: 100,
                cpus: vec![Some(0.0), Some(0.0)],
            }],
        );
        assert_eq!(list.rows[0].cores_text(), "Idle");
        list.invalidate();
        assert_eq!(list.rows[0].load_text(), "—");
        assert_eq!(list.rows[0].cores_text(), "Unavailable");
    }

    #[test]
    fn whole_machine_cpu_stays_available_without_any_scheduler_mapping() {
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("app".into()),
        };
        let mut list = ProcessList::from_tracking(
            &HashMap::from([(target, vec![(42, 100)])]),
            &HashMap::from([((42, 100), Some(2.0))]),
            &HashMap::new(),
        );
        list.record_cores(20, &[]);
        assert_eq!(list.rows[0].total, Some(2.0));
        assert_eq!(list.rows[0].known_cores, 0);
        assert!(
            list.rows[0].cores.is_empty(),
            "unknown mapping cannot invent CPU0 or affinity-based locations"
        );
        assert!(list.reserved_bytes() > 0);
    }

    #[test]
    fn current_rows_keep_idle_and_unavailable_instances_and_exact_owner_tokens() {
        let one = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("one".into()),
        };
        let two = RuleTarget {
            rule: RuleId("two".into()),
            ..one.clone()
        };
        let targets = HashMap::from([
            (one.clone(), vec![(1, 10), (1, 10), (2, 20), (3, 30)]),
            (two.clone(), vec![(1, 11)]),
        ]);
        let totals = HashMap::from([((1, 10), Some(2.0)), ((2, 20), None), ((3, 30), Some(0.0))]);
        let names = HashMap::from([(
            (1, 10),
            Arc::new(os_api::cpu_load::CpuProcess {
                pid: 1,
                instance: 10,
                name: "busy.exe".into(),
            }),
        )]);
        let mut list = ProcessList::from_tracking(&targets, &totals, &names);
        assert_eq!(
            list.rows.len(),
            4,
            "include zero and missing data, deduplicate exact owner/instance"
        );
        list.record_cores(
            2,
            &[
                os_api::cpu_load::ProcessCpuSample {
                    pid: 1,
                    token: 10,
                    cpus: vec![Some(40.0), None],
                },
                os_api::cpu_load::ProcessCpuSample {
                    pid: 3,
                    token: 30,
                    cpus: vec![Some(0.0); 2],
                },
            ],
        );
        let row = list
            .rows
            .iter()
            .find(|r| r.target == one && r.pid == 1)
            .unwrap();
        assert_eq!(row.total, Some(2.0));
        assert_eq!(row.cores, vec![(0, 40.0)]);
        assert_eq!(row.known_cores, 1, "partial mapping stays partial");
        assert_eq!(row.name(), "busy.exe");
        let missing = list.rows.iter().find(|r| r.pid == 2).unwrap();
        assert!(missing.total.is_none() && missing.cores.is_empty() && missing.known_cores == 0);
        let idle = list.rows.iter().find(|r| r.pid == 3).unwrap();
        assert_eq!(idle.known_cores, 2);
        assert!(idle.cores.is_empty());
        let reused = list.rows.iter().find(|r| r.target == two).unwrap();
        assert!(reused.total.is_none() && reused.known_cores == 0);
        assert_eq!(
            reused.name(),
            "Name unavailable",
            "never borrow an earlier PID's name"
        );
        list.invalidate();
        assert!(!list.ready);
        assert!(list
            .rows
            .iter()
            .all(|r| r.total.is_none() && r.known_cores == 0 && r.cores.is_empty()));
    }
}
