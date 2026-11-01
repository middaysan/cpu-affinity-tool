use super::model::RuleTarget;
use crate::app::shared::ids::GroupId;
use std::collections::{HashMap, HashSet};
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
    pub capacity: super::capacity::CapacityTotals,
    pub rows: Vec<ProcessRow>,
    pub rule_totals: HashMap<RuleTarget, CpuTotal>,
    pub group_totals: HashMap<GroupId, CpuTotal>,
    pub ready: bool,
    pub time: Option<SystemTime>,
    pub original_time: Option<super::history::SampleTime>,
    pub group: bool,
    pub partial: bool,
    pub cpu_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct CpuTotal {
    pub value: Option<f32>,
    pub measured: usize,
    pub instances: usize,
}
impl CpuTotal {
    fn record(&mut self, value: Option<f32>) {
        self.instances += 1;
        if let Some(value) = value.filter(|v| v.is_finite() && (0.0..=100.0).contains(v)) {
            self.measured += 1;
            self.value = Some((self.value.unwrap_or(0.0) + value).min(100.0));
        }
    }
    pub fn partial(self) -> bool {
        self.measured < self.instances
    }
}

impl ProcessList {
    pub fn displayed_time(&self, capacity_valid: bool) -> Option<super::history::SampleTime> {
        if self.group {
            self.original_time
        } else if capacity_valid {
            self.capacity.time
        } else {
            None
        }
    }
    pub fn displayed_total(&self, row: &ProcessRow, capacity_valid: bool) -> Option<f32> {
        if self.group {
            row.total
        } else {
            self.capacity_value(row, capacity_valid)
                .and_then(|v| v.value)
        }
    }
    pub fn displayed_partial(&self, row: &ProcessRow, capacity_valid: bool) -> bool {
        if self.group {
            row.partial
        } else {
            self.capacity_value(row, capacity_valid)
                .is_none_or(|v| !v.complete)
        }
    }
    fn capacity_value(
        &self,
        row: &ProcessRow,
        valid: bool,
    ) -> Option<&super::capacity::CapacityValue> {
        if !valid {
            return None;
        }
        row.target.as_ref().and_then(|t| {
            self.capacity
                .values
                .get(&(t.group.clone(), row.pid, row.token))
        })
    }
    pub fn record_capacity(
        &mut self,
        masks: &HashMap<GroupId, Vec<usize>>,
        samples: &[os_api::cpu_load::ProcessCpuSample],
        time: Option<super::history::SampleTime>,
    ) {
        self.capacity = super::capacity::CapacityTotals::build(
            self.rows
                .iter()
                .filter_map(|r| r.target.as_ref().map(|t| (t, r.pid, r.token))),
            masks,
            samples,
            time,
            self.cpu_count,
        );
    }
    pub fn rule_capacity_total(&self, target: &RuleTarget, mask: &[usize]) -> CpuTotal {
        self.capacity_total(
            self.capacity.rules.get(target).copied(),
            &target.group,
            mask,
        )
    }
    pub fn group_capacity_total(&self, group: &GroupId, mask: &[usize]) -> CpuTotal {
        self.capacity_total(self.capacity.groups.get(group).copied(), group, mask)
    }
    fn capacity_total(&self, total: Option<CpuTotal>, group: &GroupId, mask: &[usize]) -> CpuTotal {
        let total = total.unwrap_or_default();
        if self.ready && !self.group && self.capacity.matches(group, mask) {
            total
        } else {
            CpuTotal {
                instances: total.instances,
                ..Default::default()
            }
        }
    }
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
        let mut rule_totals = HashMap::<RuleTarget, CpuTotal>::new();
        let mut group_totals = HashMap::<GroupId, CpuTotal>::new();
        let mut group_instances = HashMap::<GroupId, HashSet<(u32, u64)>>::new();
        for row in &rows {
            let Some(target) = row.target.as_ref() else {
                continue;
            };
            rule_totals
                .entry(target.clone())
                .or_default()
                .record(row.total);
            if group_instances
                .entry(target.group.clone())
                .or_default()
                .insert((row.pid, row.token))
            {
                group_totals
                    .entry(target.group.clone())
                    .or_default()
                    .record(row.total);
            }
        }
        Self {
            rows,
            rule_totals,
            group_totals,
            ready: true,
            time: None,
            ..Default::default()
        }
    }

    pub fn from_contributions(
        time: super::history::SampleTime,
        cpu_count: usize,
        known_cores: usize,
        partial: bool,
        contributions: impl Iterator<Item = (Arc<os_api::cpu_load::CpuProcess>, usize, f32)>,
    ) -> Self {
        let mut list = Self {
            ready: true,
            time: Some(time.wall),
            original_time: Some(time),
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
        self.cpu_count = count;
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
        self.capacity = Default::default();
        self.rule_totals.clear();
        self.group_totals.clear();
        for row in &mut self.rows {
            row.total = None;
            row.known_cores = 0;
            row.cores.clear();
            row.load_text = super::view::percent(None);
            row.update_cores_text();
        }
    }
    pub fn reserved_bytes(&self) -> usize {
        self.capacity.reserved_bytes()
            + self.rows.capacity() * std::mem::size_of::<ProcessRow>()
            + self.rule_totals.capacity() * std::mem::size_of::<(RuleTarget, CpuTotal)>()
            + self.group_totals.capacity() * std::mem::size_of::<(GroupId, CpuTotal)>()
            + self
                .group_totals
                .keys()
                .map(|group| group.0.capacity())
                .sum::<usize>()
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

    /// The exact member union uses the same current counters as application rows.
    #[cfg(test)]
    pub fn group_cpu_total(&self, group: &GroupId) -> CpuTotal {
        if !self.ready || self.group {
            return CpuTotal::default();
        }
        self.group_totals.get(group).copied().unwrap_or(CpuTotal {
            value: Some(0.0),
            ..Default::default()
        })
    }

    /// Current process-time coverage, independent of per-CPU ETW mapping.
    #[cfg(test)]
    pub fn rule_cpu_total(&self, target: &RuleTarget) -> CpuTotal {
        if !self.ready || self.group {
            return CpuTotal::default();
        }
        self.rule_totals.get(target).copied().unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub(super) struct CurrentOrder {
    source: Option<Arc<ProcessList>>,
    target: Option<RuleTarget>,
    capacity_valid: bool,
    revision: u64,
    order: crate::app::shell::presenters::process_table::Order,
}

pub(super) fn draw(
    ui: &mut eframe::egui::Ui,
    list: &Arc<ProcessList>,
    target: Option<&RuleTarget>,
    available_height: f32,
    mask: &[usize],
) {
    crate::render_profile_scope!(CurrentProcesses);

    use crate::app::shell::presenters::shared_elements::{
        compact_table_spacing, palette, table_row_separator, COMPACT_TABLE_ROW_HEIGHT,
    };
    use eframe::egui::{self, RichText};
    let colors = *palette(ui);
    let top = ui.cursor().top();
    let capacity_valid =
        list.ready && target.is_some_and(|t| list.capacity.matches(&t.group, mask));
    let mapping_current = list.group || (capacity_valid && list.capacity.time.is_some());
    let rows = || {
        list.rows
            .iter()
            .filter(|row| target.is_none_or(|target| row.target.as_ref() == Some(target)))
    };
    let count = rows().count();
    let partial = list.partial || rows().any(|r| list.displayed_partial(r, capacity_valid));
    let clock = list
        .displayed_time(capacity_valid)
        .map(|t| t.wall)
        .map(os_api::cpu_load::sample_clock_label)
        .unwrap_or_else(|| "—".into());
    ui.label(
        RichText::new(format!(
            "{count} {} · Latest · {clock}",
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
        let total = target
            .map(|t| list.rule_capacity_total(t, mask))
            .unwrap_or_default();
        format!(
            "{} group CPUs · {}/{} processes complete",
            super::capacity::normalized_mask(mask).len(),
            total.measured,
            total.instances
        )
    } else {
        "Updating tracked processes…".into()
    };
    ui.label(RichText::new(status).small().color(if !list.ready||partial {colors.warning.fg}else{colors.text_secondary}))
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
            if i==2 {response.on_hover_text(if list.group {"This process's measured share of the displayed CPUs. 100% means all displayed CPUs fully busy. Only work on these CPUs is counted; * marks a measured subtotal with missing data. Uses the same original interval as the graphs."} else {"This process's measured share of the group's selected CPU capacity, using the same completed scheduler interval as the app/group meters. 100% means all selected CPUs fully busy. Work outside the mask is excluded; * marks incomplete selected-CPU coverage. Raw whole-host counters remain separate in the MCP API."});}
            else if i==3 {response.on_hover_text("Logical CPUs with measured positive work in the latest scheduler interval. A process can run on several CPUs; this is not a single instantaneous location or its allowed affinity. Idle means a complete measured zero interval; Unavailable and * preserve missing/partial data.");}
        });
        let indices = ui.ctx().data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<CurrentOrder>(id.with("cache"));
            if cache.source.as_ref().is_none_or(|old| !Arc::ptr_eq(old,list)) || cache.target.as_ref()!=target || cache.capacity_valid!=capacity_valid {
                cache.capacity_valid=capacity_valid;
                cache.source=Some(list.clone()); cache.target=target.cloned(); cache.revision=cache.revision.wrapping_add(1);
            }
            cache.order.sort=order.sort;
            let sort=order.sort.unwrap_or(Sort{column:Column::Pid,descending:false});
            cache.order.prepare(cache.revision,list.rows.len(),|i| target.is_none_or(|target| list.rows[i].target.as_ref()==Some(target)),|a,b| {
                let (a,b)=(&list.rows[a],&list.rows[b]);
                let primary=match sort.column {Column::Name=>sort.compare(table::name_cmp(a.name(),b.name())),Column::Load=>sort.load(list.displayed_total(a,capacity_valid),list.displayed_total(b,capacity_valid)),_=>sort.compare(a.pid.cmp(&b.pid))};
                primary.then(a.pid.cmp(&b.pid)).then(a.token.cmp(&b.token))
            });
            cache.order.rows.clone()
        });
        ui.ctx().data_mut(|d| d.insert_temp(id,order));
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        super::debug_api::record_processes(ui.ctx(), list, &indices, capacity_valid);
        ui.separator();
        let height=(available_height-(ui.cursor().top()-top)).max(60.0);
        let scroll = egui::ScrollArea::vertical().id_salt("cpu-process-list").min_scrolled_height(height).max_height(height).auto_shrink([false,false]).show_rows(ui,COMPACT_TABLE_ROW_HEIGHT,count,|ui,range| {
            if count==0 {ui.label(if list.group {if !list.ready {"No process data for this interval."} else if list.cpu_count==0 {"No CPUs selected. Enable All CPUs."} else if list.partial {"No measured activity; some data is unavailable."} else {"No measured process activity on these CPUs."}} else if list.ready{"No tracked processes for this app."}else{"Waiting for the first tracked-process snapshot…"});}
            for &index in &indices[range] {
                let row=&list.rows[index];
                let row_response = ui.push_id((row.pid,row.token),|ui|ui.horizontal(|ui| {
                    cell(ui,RichText::new(row.name()).small(),widths[0],false).on_hover_text(row.name());
                    cell(ui,RichText::new(row.pid_text()).small(),widths[1],true);
                    let text = if list.group {row.load_text()} else {list.capacity_value(row,capacity_valid).map_or("—",|v|v.text.as_str())};
                    cell(ui,RichText::new(text).small(),widths[2],true);
                    let complete=mapping_current&&row.cpu_count>0&&row.known_cores==row.cpu_count&&!row.partial;
                    cell(ui,RichText::new(if mapping_current {row.cores_text()} else {"Unavailable"}).small().color(if complete{colors.text_primary}else{colors.warning.fg}),widths[3],false).on_hover_ui(|ui| {
                        ui.label(format!("CPU mapping: {}/{} logical CPUs measured",row.known_cores,row.cpu_count));
                        if mapping_current { for &(cpu,load) in &row.cores {ui.label(format!("CPU{cpu}: {}",super::view::percent(Some(load))));} }
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
    fn s135_capacity_counts_selected_cpus_and_exact_group_union() {
        let one = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("one".into()),
        };
        let shared = RuleTarget {
            rule: RuleId("shared".into()),
            ..one.clone()
        };
        let other = RuleTarget {
            group: GroupId("other".into()),
            ..one.clone()
        };
        let targets = HashMap::from([
            (one.clone(), vec![(1, 10), (2, 20), (2, 20)]),
            (shared.clone(), vec![(2, 20)]),
            (other.clone(), vec![(1, 10), (2, 20)]),
        ]);
        let raw = HashMap::from([((1, 10), Some(11.0)), ((2, 20), Some(8.0))]);
        let mut list = ProcessList::from_tracking(&targets, &raw, &Default::default());
        let mut main = vec![None; 20];
        let mut helper = vec![None; 20];
        main[..6].fill(Some(20.0));
        helper[..6].fill(Some(10.0));
        main[18] = Some(0.0);
        main[19] = Some(100.0);
        helper[18] = Some(100.0);
        helper[19] = Some(0.0);
        let samples = [
            os_api::cpu_load::ProcessCpuSample {
                pid: 1,
                token: 10,
                cpus: main,
            },
            os_api::cpu_load::ProcessCpuSample {
                pid: 2,
                token: 20,
                cpus: helper,
            },
        ];
        let masks = HashMap::from([
            (one.group.clone(), (0..6).collect()),
            (other.group.clone(), vec![18, 19]),
        ]);
        list.record_cores(20, &samples);
        list.record_capacity(
            &masks,
            &samples,
            Some(super::super::history::SampleTime {
                seconds: 1.0,
                wall: SystemTime::UNIX_EPOCH,
            }),
        );
        assert_eq!(
            list.rule_capacity_total(&one, &[0, 1, 2, 3, 4, 5]),
            CpuTotal {
                value: Some(30.0),
                measured: 2,
                instances: 2
            }
        );
        assert_eq!(
            list.rule_capacity_total(&shared, &[0, 1, 2, 3, 4, 5]).value,
            Some(10.0)
        );
        assert_eq!(
            list.group_capacity_total(&one.group, &[0, 1, 2, 3, 4, 5])
                .value,
            Some(30.0),
            "the helper shared by two rules must count once"
        );
        assert_eq!(
            list.group_capacity_total(&other.group, &[18, 19]).value,
            Some(100.0)
        );
        assert_eq!(
            list.rows[0].total,
            Some(11.0),
            "explicit whole-host API counters must retain their units"
        );
    }

    #[test]
    fn s135_capacity_preserves_unknown_selected_cells_lifetimes_and_mask_edits() {
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let mut list = ProcessList::from_tracking(
            &HashMap::from([(target.clone(), vec![(1, 10), (1, 11)])]),
            &HashMap::from([((1, 10), Some(90.0)), ((1, 11), Some(80.0))]),
            &Default::default(),
        );
        let samples = [os_api::cpu_load::ProcessCpuSample {
            pid: 1,
            token: 10,
            cpus: vec![Some(60.0), None, Some(100.0)],
        }];
        let mut masks = HashMap::from([(target.group.clone(), vec![0, 1])]);
        let time = Some(super::super::history::SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        });
        list.record_cores(3, &samples);
        list.record_capacity(&masks, &samples, time);
        let partial = list.rule_capacity_total(&target, &[1, 0, 0]);
        assert_eq!(
            partial.value,
            Some(30.0),
            "unknown selected CPUs keep the full denominator"
        );
        assert_eq!((partial.measured, partial.instances), (0, 2));
        assert!(partial.partial());
        assert_eq!(
            list.rule_capacity_total(&target, &[0]).value,
            None,
            "a changed saved mask cannot reuse another scope"
        );
        masks.insert(target.group.clone(), vec![2]);
        list.record_capacity(&masks, &samples, time);
        assert_eq!(list.rule_capacity_total(&target, &[2]).value, Some(100.0));
        list.record_capacity(&masks, &[], None);
        assert_eq!(
            list.rule_capacity_total(&target, &[2]).value,
            None,
            "trace loss must not fall back to raw whole-host percentages"
        );
        masks.insert(target.group.clone(), vec![]);
        list.record_capacity(&masks, &samples, time);
        assert_eq!(list.group_capacity_total(&target.group, &[]).value, None);
        list.invalidate();
        assert_eq!(list.rule_capacity_total(&target, &[2]).value, None);
    }

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
            CpuTotal {
                value: Some(5.0),
                measured: 2,
                instances: 4
            }
        );
        assert!(list.rule_cpu_total(&one).partial());
        assert_eq!(
            list.rule_cpu_total(&two),
            CpuTotal {
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
        assert_eq!(list.rule_cpu_total(&one), CpuTotal::default());
        totals.insert((3, 30), Some(2.0));
        totals.insert((4, 40), Some(3.0));
        let recovered = ProcessList::from_tracking(&targets, &totals, &Default::default());
        assert_eq!(
            recovered.rule_cpu_total(&one),
            CpuTotal {
                value: Some(10.0),
                measured: 4,
                instances: 4
            }
        );
        assert!(!recovered.rule_cpu_total(&one).partial());
        assert_eq!(
            ProcessList::from_tracking(&Default::default(), &totals, &Default::default())
                .rule_cpu_total(&one),
            CpuTotal::default()
        );
    }

    #[test]
    fn s118_group_cpu_counts_exact_members_once_and_keeps_coverage_current() {
        let target = |group: &str, rule: &str| RuleTarget {
            group: GroupId(group.into()),
            rule: RuleId(rule.into()),
        };
        let g = GroupId("g".into());
        let h = GroupId("h".into());
        let targets = HashMap::from([
            (
                target("g", "main-and-extra"),
                vec![(1, 10), (2, 20), (2, 20)],
            ),
            (target("g", "overlap"), vec![(2, 20)]),
            (target("h", "shared-and-reused"), vec![(2, 20), (2, 21)]),
        ]);
        let mut totals = HashMap::from([((1, 10), Some(6.0)), ((2, 20), Some(4.0))]);
        let mut list = ProcessList::from_tracking(&targets, &totals, &Default::default());
        assert_eq!(
            list.group_cpu_total(&g),
            CpuTotal {
                value: Some(10.0),
                measured: 2,
                instances: 2
            }
        );
        assert_eq!(
            list.group_cpu_total(&h),
            CpuTotal {
                value: Some(4.0),
                measured: 1,
                instances: 2
            }
        );
        assert!(list.group_cpu_total(&h).partial());
        assert_eq!(
            list.group_cpu_total(&GroupId("empty".into())).value,
            Some(0.0)
        );
        list.record_cores(20, &[]);
        assert_eq!(
            list.group_cpu_total(&g).value,
            Some(10.0),
            "ETW is independent of current totals"
        );
        list.invalidate();
        assert_eq!(list.group_cpu_total(&g), CpuTotal::default());
        assert_eq!(
            list.group_cpu_total(&GroupId("empty".into())),
            CpuTotal::default()
        );
        totals.insert((2, 21), Some(3.0));
        let recovered = ProcessList::from_tracking(&targets, &totals, &Default::default());
        assert_eq!(
            recovered.group_cpu_total(&h),
            CpuTotal {
                value: Some(7.0),
                measured: 2,
                instances: 2
            }
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
                |ui| draw(ui, &list, Some(&target), 330.0, &[0, 1]),
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
