use super::model::RuleTarget;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

pub(super) struct ProcessRow {
    pub target: Option<RuleTarget>,
    pub pid: u32,
    pub token: u64,
    identity: Option<Arc<os_api::cpu_load::CpuProcess>>,
    /// Whole-machine CPU time, independent of scheduler attribution.
    pub total: Option<f32>,
    pub cores: Vec<(usize, f32)>,
    pub known_cores: usize,
    pub cpu_count: usize,
    pub partial: bool,
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
    pub fn mapping_partial(&self) -> bool {
        self.cpu_count == 0 || self.known_cores != self.cpu_count || self.partial
    }
    fn update_cores_text(&mut self) {
        use std::fmt::Write;
        self.cores_text.clear();
        let complete = !self.mapping_partial();
        if self.known_cores == 0 && self.cores.is_empty() {
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
    pub rule_totals: HashMap<RuleTarget, RuleCpuTotal>,
    pub ready: bool,
    pub time: Option<SystemTime>,
    pub group: bool,
    pub partial: bool,
    pub cpu_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct RuleCpuTotal {
    pub value: Option<f32>,
    pub measured: usize,
    pub instances: usize,
}
impl RuleCpuTotal {
    pub fn partial(self) -> bool {
        self.measured < self.instances
    }
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
                    target: Some(target.clone()),
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
                    partial: false,
                    pid_text: pid.to_string(),
                    load_text: super::view::percent(totals.get(&(pid, token)).copied().flatten()),
                    cores_text: "Unavailable".into(),
                })
            })
            .collect();
        rows.sort_unstable_by(|a, b| {
            a.pid.cmp(&b.pid).then(a.token.cmp(&b.token)).then_with(|| {
                a.target
                    .as_ref()
                    .map(|t| (&t.group.0, &t.rule.0))
                    .cmp(&b.target.as_ref().map(|t| (&t.group.0, &t.rule.0)))
            })
        });
        rows.dedup_by(|a, b| a.pid == b.pid && a.token == b.token && a.target == b.target);
        let mut rule_totals = HashMap::<RuleTarget, RuleCpuTotal>::new();
        for row in &rows {
            let Some(target) = row.target.as_ref() else {
                continue;
            };
            let total = rule_totals.entry(target.clone()).or_default();
            total.instances += 1;
            if let Some(value) = row
                .total
                .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
            {
                total.measured += 1;
                total.value = Some((total.value.unwrap_or(0.0) + value).min(100.0));
            }
        }
        Self {
            rows,
            rule_totals,
            ready: true,
            time: None,
            ..Default::default()
        }
    }

    pub fn from_contributions(
        time: SystemTime,
        cpu_count: usize,
        known_cores: usize,
        partial: bool,
        contributions: impl Iterator<Item = (Arc<os_api::cpu_load::CpuProcess>, usize, f32)>,
    ) -> Self {
        let mut list = Self {
            ready: true,
            time: Some(time),
            group: true,
            partial,
            cpu_count,
            ..Default::default()
        };
        let mut by_instance = HashMap::new();
        for (process, cpu, load) in contributions {
            let index = *by_instance
                .entry((process.pid, process.instance))
                .or_insert_with(|| {
                    let index = list.rows.len();
                    list.rows.push(ProcessRow {
                        target: None,
                        pid: process.pid,
                        token: process.instance,
                        pid_text: process.pid.to_string(),
                        identity: Some(process),
                        total: None,
                        cores: Vec::new(),
                        known_cores,
                        cpu_count,
                        partial,
                        load_text: String::new(),
                        cores_text: String::new(),
                    });
                    index
                });
            let row = &mut list.rows[index];
            match row.cores.binary_search_by_key(&cpu, |&(cpu, _)| cpu) {
                // Multiple saved rules can own the same exact PID/CPU sample.
                Ok(index) => row.cores[index].1 = row.cores[index].1.max(load),
                Err(index) => row.cores.insert(index, (cpu, load)),
            }
        }
        for row in &mut list.rows {
            row.total = (cpu_count > 0)
                .then(|| row.cores.iter().map(|&(_, v)| v).sum::<f32>() / cpu_count as f32);
            row.load_text = super::view::measured_percent(row.total, partial);
            row.update_cores_text();
        }
        list.rows.sort_unstable_by_key(|row| (row.pid, row.token));
        list
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
        self.rule_totals.clear();
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
            + self.rule_totals.capacity() * std::mem::size_of::<(RuleTarget, RuleCpuTotal)>()
            + self
                .rule_totals
                .keys()
                .map(|target| target.group.0.capacity() + target.rule.0.capacity())
                .sum::<usize>()
            + self
                .rows
                .iter()
                .map(|row| {
                    row.cores.capacity() * std::mem::size_of::<(usize, f32)>()
                        + row.target.as_ref().map_or(0, |target| {
                            target.group.0.capacity() + target.rule.0.capacity()
                        })
                        + row.pid_text.capacity()
                        + row.load_text.capacity()
                        + row.cores_text.capacity()
                })
                .sum::<usize>()
    }

    /// Current process-time coverage, independent of per-CPU ETW mapping.
    pub fn rule_cpu_total(&self, target: &RuleTarget) -> RuleCpuTotal {
        if !self.ready || self.group {
            return RuleCpuTotal::default();
        }
        self.rule_totals.get(target).copied().unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub(super) struct CurrentOrder {
    source: Option<Arc<ProcessList>>,
    target: Option<RuleTarget>,
    revision: u64,
    order: crate::app::shell::presenters::process_table::Order,
}

pub(super) fn draw(
    ui: &mut eframe::egui::Ui,
    list: &Arc<ProcessList>,
    target: Option<&RuleTarget>,
    available_height: f32,
) {
    crate::render_profile_scope!(CurrentProcesses);

    use crate::app::shell::presenters::shared_elements::{
        compact_table_spacing, palette, table_row_separator, COMPACT_TABLE_ROW_HEIGHT,
    };
    use eframe::egui::{self, RichText};
    let colors = *palette(ui);
    let top = ui.cursor().top();
    let rows = || {
        list.rows
            .iter()
            .filter(|row| target.is_none_or(|target| row.target.as_ref() == Some(target)))
    };
    let count = rows().count();
    let mapped = rows().filter(|r| !r.mapping_partial()).count();
    let clock = list
        .time
        .map(os_api::cpu_load::sample_clock_label)
        .unwrap_or_else(|| "—".into());
    ui.label(
        RichText::new(format!(
            "{count} {} · Live · {clock}",
            if list.group {
                "processes"
            } else {
                "tracked processes"
            }
        ))
        .small()
        .strong(),
    );
    let status = if list.group {
        if list.ready {
            format!(
                "Load on {} displayed CPUs{}",
                list.cpu_count,
                if list.partial { " · Partial" } else { "" }
            )
        } else {
            "Process data unavailable for this interval".into()
        }
    } else if list.ready {
        format!("CPU mapping: {mapped}/{count} processes")
    } else {
        "Updating tracked processes…".into()
    };
    ui.label(RichText::new(status).small().color(if !list.ready||list.partial||mapped<count {colors.warning.fg}else{colors.text_secondary}))
        .on_hover_text(if list.group {"Measured work on the displayed CPUs in the latest completed graph interval. Idle processes have no measured CPU location. Partial means some data is missing; an empty list is not a complete process roster."} else {"The Process monitor roster and this table use tracked exact PID/creation-token instances. Refresh times can differ. This current table includes idle and unavailable instances; graph contributors include only measured work on the selected CPU/time. The app-row CPU value shows a measured subtotal with * when a sibling is unavailable; its complete aggregate remains unknown."});
    let widths = [(ui.available_width() - 202.0).max(46.0), 46.0, 46.0, 86.0];
    use crate::app::shell::presenters::process_table::{self as table, Column, Sort};
    let cell = |ui: &mut egui::Ui, text: RichText, width: f32, right: bool| {
        table::cell(ui, width, right, text)
    };
    let id = egui::Id::new("current-process-sort");
    let mut order = ui
        .ctx()
        .data(|d| d.get_temp::<table::Order>(id))
        .unwrap_or_default();
    ui.scope(|ui| {
        compact_table_spacing(ui);
        ui.spacing_mut().item_spacing.x=6.0;
        ui.horizontal(|ui|for (i,title) in ["Process","PID","CPU %","CPUs"].iter().enumerate(){
            let response=if i<3 {table::heading(ui,widths[i],title,[Column::Name,Column::Pid,Column::Load][i], &mut order)} else {cell(ui,RichText::new(*title).small().weak(),widths[i],false)};
            #[cfg(all(debug_assertions, feature = "debug-mcp"))]
            crate::app::features::diagnostics::debug_mcp::control(format!("sort.processes.{i}"), &response);
            if i==2 {response.on_hover_text(if list.group {"This process's measured share of the displayed CPUs. 100% means all displayed CPUs fully busy. Only work on these CPUs is counted; * marks a measured subtotal with missing data. Uses the same original interval as the graphs."} else {"Process CPU time relative to the whole machine, sampled once per second with GetProcessTimes. This can remain available when scheduler/per-core attribution is unavailable. Its interval is not identical to Task Manager or the per-CPU trace."});}
            else if i==3 {response.on_hover_text("Logical CPUs with measured positive work in the latest scheduler interval. A process can run on several CPUs; this is not a single instantaneous location or its allowed affinity. Idle means a complete measured zero interval; Unavailable and * preserve missing/partial data.");}
        });
        let indices = ui.ctx().data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<CurrentOrder>(id.with("cache"));
            if cache.source.as_ref().is_none_or(|old| !Arc::ptr_eq(old,list)) || cache.target.as_ref()!=target {
                cache.source=Some(list.clone()); cache.target=target.cloned(); cache.revision=cache.revision.wrapping_add(1);
            }
            cache.order.sort=order.sort;
            let sort=order.sort.unwrap_or(Sort{column:Column::Pid,descending:false});
            cache.order.prepare(cache.revision,list.rows.len(),|i| target.is_none_or(|target| list.rows[i].target.as_ref()==Some(target)),|a,b| {
                let (a,b)=(&list.rows[a],&list.rows[b]);
                let primary=match sort.column {Column::Name=>sort.compare(table::name_cmp(a.name(),b.name())),Column::Load=>sort.load(a.total,b.total),_=>sort.compare(a.pid.cmp(&b.pid))};
                primary.then(a.pid.cmp(&b.pid)).then(a.token.cmp(&b.token))
            });
            cache.order.rows.clone()
        });
        ui.ctx().data_mut(|d| d.insert_temp(id,order));
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        super::debug_api::record_processes(ui.ctx(), list, &indices);
        ui.separator();
        let height=(available_height-(ui.cursor().top()-top)).max(60.0);
        let scroll = egui::ScrollArea::vertical().id_salt("cpu-process-list").min_scrolled_height(height).max_height(height).auto_shrink([false,false]).show_rows(ui,COMPACT_TABLE_ROW_HEIGHT,count,|ui,range| {
            if count==0 {ui.label(if list.group {if !list.ready {"No process data for this interval."} else if list.cpu_count==0 {"No CPUs selected. Enable All CPUs."} else if list.partial {"No measured activity; some data is unavailable."} else {"No measured process activity on these CPUs."}} else if list.ready{"No tracked processes for this app."}else{"Waiting for the first tracked-process snapshot…"});}
            for &index in &indices[range] {
                let row=&list.rows[index];
                let row_response = ui.push_id((row.pid,row.token),|ui|ui.horizontal(|ui| {
                    cell(ui,RichText::new(row.name()).small(),widths[0],false).on_hover_text(row.name());
                    cell(ui,RichText::new(row.pid_text()).small(),widths[1],true);
                    cell(ui,RichText::new(row.load_text()).small(),widths[2],true);
                    let complete=row.cpu_count>0&&row.known_cores==row.cpu_count&&!row.partial;
                    cell(ui,RichText::new(row.cores_text()).small().color(if complete{colors.text_primary}else{colors.warning.fg}),widths[3],false).on_hover_ui(|ui| {
                        ui.label(format!("CPU mapping: {}/{} logical CPUs measured",row.known_cores,row.cpu_count));
                        for &(cpu,load) in &row.cores {ui.label(format!("CPU{cpu}: {}",super::view::percent(Some(load))));}
                    });
                }));
                table_row_separator(ui, row_response.inner.response.rect);
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
    fn cached_cpu_subtotals_deduplicate_exact_owners_and_expire_with_the_snapshot() {
        let one = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("one".into()),
        };
        let two = RuleTarget {
            rule: RuleId("two".into()),
            ..one.clone()
        };
        let targets = HashMap::from([
            (
                one.clone(),
                vec![(1, 10), (1, 10), (2, 20), (3, 30), (4, 40)],
            ),
            (two.clone(), vec![(1, 10), (1, 11)]),
        ]);
        let mut totals = HashMap::from([
            ((1, 10), Some(5.0)),
            ((2, 20), Some(0.0)),
            ((3, 30), Some(f32::NAN)),
            ((4, 40), Some(-1.0)),
        ]);
        let mut list = ProcessList::from_tracking(&targets, &totals, &Default::default());
        assert_eq!(
            list.rule_cpu_total(&one),
            RuleCpuTotal {
                value: Some(5.0),
                measured: 2,
                instances: 4
            }
        );
        assert!(list.rule_cpu_total(&one).partial());
        assert_eq!(
            list.rule_cpu_total(&two),
            RuleCpuTotal {
                value: Some(5.0),
                measured: 1,
                instances: 2
            }
        );
        list.record_cores(20, &[]);
        assert_eq!(
            list.rule_cpu_total(&one).value,
            Some(5.0),
            "missing ETW mapping must not erase process-time coverage"
        );
        list.invalidate();
        assert_eq!(list.rule_cpu_total(&one), RuleCpuTotal::default());
        totals.insert((3, 30), Some(2.0));
        totals.insert((4, 40), Some(3.0));
        let recovered = ProcessList::from_tracking(&targets, &totals, &Default::default());
        assert_eq!(
            recovered.rule_cpu_total(&one),
            RuleCpuTotal {
                value: Some(10.0),
                measured: 4,
                instances: 4
            }
        );
        assert!(!recovered.rule_cpu_total(&one).partial());
        assert_eq!(
            ProcessList::from_tracking(&Default::default(), &totals, &Default::default())
                .rule_cpu_total(&one),
            RuleCpuTotal::default()
        );
    }

    #[test]
    fn compact_current_table_paints_dense_rows_and_separators() {
        use eframe::egui;
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let list = ProcessList::from_tracking(
            &HashMap::from([(target.clone(), (1000..1040).map(|pid| (pid, 1)).collect())]),
            &HashMap::new(),
            &HashMap::new(),
        );
        let list = Arc::new(list);
        let ctx = egui::Context::default();
        let mut output = egui::FullOutput::default();
        for _ in 0..3 {
            output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| draw(ui, &list, Some(&target), 330.0),
            );
            output.textures_delta.clear();
        }
        let positions = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t)
                    if t.galley
                        .text()
                        .parse::<u32>()
                        .is_ok_and(|pid| (1000..1040).contains(&pid)) =>
                {
                    Some(t.pos.y)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(positions.len() > 10);
        assert!(
            positions[1] - positions[0] <= 15.1,
            "current rows must be compact: {:?}",
            &positions[..2]
        );
        let lines=output.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::LineSegment{points,stroke} if (points[0].y-points[1].y).abs()<0.1 && (points[1].x-points[0].x).abs()>250.0 && stroke.width<=1.0)).count();
        assert!(
            lines >= positions.len() - 1,
            "each visible current row needs a subtle separator"
        );
    }

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
            .find(|r| r.target.as_ref() == Some(&one) && r.pid == 1)
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
        let reused = list
            .rows
            .iter()
            .find(|r| r.target.as_ref() == Some(&two))
            .unwrap();
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
