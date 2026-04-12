use crate::ir::TimingSummary;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, time::Duration};

pub type ReportMetrics = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageLogLevel {
    Info,
    Warning,
    Progress,
}

impl StageLogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Progress => "progress",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageEvent {
    Started {
        stage: &'static str,
    },
    Log {
        stage: &'static str,
        level: StageLogLevel,
        message: String,
    },
    Finished {
        stage: &'static str,
        status: ReportStatus,
        elapsed_ms: u64,
    },
}

pub trait StageReporter {
    fn on_stage_event(&mut self, event: StageEvent);

    fn is_cancelled(&self) -> bool {
        false
    }
}

impl<F> StageReporter for F
where
    F: FnMut(StageEvent),
{
    fn on_stage_event(&mut self, event: StageEvent) {
        self(event);
    }
}

pub struct LineStageReporter<'a> {
    logger: &'a mut dyn FnMut(String),
    include_lifecycle: bool,
    include_stage_prefix: bool,
}

impl<'a> LineStageReporter<'a> {
    pub fn runtime_only(logger: &'a mut dyn FnMut(String)) -> Self {
        Self {
            logger,
            include_lifecycle: false,
            include_stage_prefix: false,
        }
    }

    pub fn with_lifecycle(logger: &'a mut dyn FnMut(String)) -> Self {
        Self {
            logger,
            include_lifecycle: true,
            include_stage_prefix: false,
        }
    }

    pub fn cli(logger: &'a mut dyn FnMut(String)) -> Self {
        Self {
            logger,
            include_lifecycle: true,
            include_stage_prefix: true,
        }
    }
}

impl StageReporter for LineStageReporter<'_> {
    fn on_stage_event(&mut self, event: StageEvent) {
        let Some(line) =
            format_stage_event_line(&event, self.include_lifecycle, self.include_stage_prefix)
        else {
            return;
        };
        (self.logger)(line);
    }
}

pub fn emit_stage_started(reporter: &mut Option<&mut dyn StageReporter>, stage: &'static str) {
    if let Some(reporter) = reporter.as_deref_mut() {
        reporter.on_stage_event(StageEvent::Started { stage });
    }
}

pub fn emit_stage_finished(
    reporter: &mut Option<&mut dyn StageReporter>,
    stage: &'static str,
    status: ReportStatus,
    elapsed: Duration,
) {
    if let Some(reporter) = reporter.as_deref_mut() {
        reporter.on_stage_event(StageEvent::Finished {
            stage,
            status,
            elapsed_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
        });
    }
}

pub fn emit_stage_info(
    reporter: &mut Option<&mut dyn StageReporter>,
    stage: &'static str,
    message: impl Into<String>,
) {
    emit_stage_event(reporter, stage, StageLogLevel::Info, message);
}

pub fn emit_stage_warning(
    reporter: &mut Option<&mut dyn StageReporter>,
    stage: &'static str,
    message: impl Into<String>,
) {
    emit_stage_event(reporter, stage, StageLogLevel::Warning, message);
}

pub fn emit_stage_progress(
    reporter: &mut Option<&mut dyn StageReporter>,
    stage: &'static str,
    message: impl Into<String>,
) {
    emit_stage_event(reporter, stage, StageLogLevel::Progress, message);
}

fn emit_stage_event(
    reporter: &mut Option<&mut dyn StageReporter>,
    stage: &'static str,
    level: StageLogLevel,
    message: impl Into<String>,
) {
    if let Some(reporter) = reporter.as_deref_mut() {
        reporter.on_stage_event(StageEvent::Log {
            stage,
            level,
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    #[default]
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StageReport {
    pub stage: String,
    #[serde(default)]
    pub status: ReportStatus,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub messages: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub metrics: ReportMetrics,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

impl StageReport {
    pub fn new(stage: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            status: ReportStatus::Success,
            elapsed_ms: None,
            messages: Vec::new(),
            warnings: Vec::new(),
            metrics: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());
    }

    pub fn warn(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.elapsed_ms = Some(elapsed.as_millis().try_into().unwrap_or(u64::MAX));
    }

    pub fn metric(&mut self, key: impl Into<String>, value: impl Serialize) {
        self.metrics.insert(
            key.into(),
            serde_json::to_value(value).expect("stage metric must serialize"),
        );
    }

    pub fn artifact(&mut self, key: impl Into<String>, path: impl AsRef<Path>) {
        self.artifacts
            .insert(key.into(), path.as_ref().display().to_string());
    }
}

#[derive(Debug, Clone)]
pub struct StageOutput<T> {
    pub value: T,
    pub report: StageReport,
}

pub fn run_stage_with_reporter<T, E, Run, RunWithReporter>(
    stage: &'static str,
    reporter: &mut Option<&mut dyn StageReporter>,
    run: Run,
    run_with_reporter: RunWithReporter,
) -> Result<StageOutput<T>, E>
where
    Run: FnOnce() -> Result<StageOutput<T>, E>,
    RunWithReporter: FnOnce(&mut dyn StageReporter) -> Result<StageOutput<T>, E>,
{
    let started_at = std::time::Instant::now();
    emit_stage_started(reporter, stage);
    let result = match reporter.as_deref_mut() {
        Some(reporter) => run_with_reporter(reporter),
        None => run(),
    };
    let elapsed = started_at.elapsed();

    match result {
        Ok(mut output) => {
            output.report.set_elapsed(elapsed);
            emit_stage_finished(reporter, stage, output.report.status, elapsed);
            Ok(output)
        }
        Err(err) => {
            emit_stage_finished(reporter, stage, ReportStatus::Failed, elapsed);
            Err(err)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImplementationReport {
    pub schema_version: u32,
    pub flow: String,
    pub design: String,
    pub out_dir: String,
    pub seed: u64,
    #[serde(default)]
    pub status: ReportStatus,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
    #[serde(default)]
    pub stages: Vec<StageReport>,
    #[serde(default)]
    pub timing: Option<TimingSummary>,
    #[serde(default)]
    pub bitstream_sha256: Option<String>,
}

pub fn print_stage_report(report: &StageReport) {
    let elapsed = report
        .elapsed_ms
        .map(format_elapsed_ms)
        .unwrap_or_else(|| "-".to_string());
    let metrics = format_metric_pairs(&report.metrics);
    if metrics.is_empty() {
        println!(
            "[{}] {} elapsed={}",
            report.stage,
            format_status(report.status),
            elapsed
        );
    } else {
        println!(
            "[{}] {} elapsed={} {}",
            report.stage,
            format_status(report.status),
            elapsed,
            metrics
        );
    }
    for warning in &report.warnings {
        println!("[{}][warn] {}", report.stage, warning);
    }
    for message in &report.messages {
        println!("[{}] {}", report.stage, message);
    }
}

pub fn format_stage_event_line(
    event: &StageEvent,
    include_lifecycle: bool,
    include_stage_prefix: bool,
) -> Option<String> {
    match event {
        StageEvent::Started { stage } if include_lifecycle && include_stage_prefix => {
            Some(format!("[{stage}] >>> starting {stage}\n"))
        }
        StageEvent::Started { stage } if include_lifecycle => {
            Some(format!(">>> starting {stage}\n"))
        }
        StageEvent::Started { .. } => None,
        StageEvent::Log {
            stage,
            level,
            message,
        } if include_stage_prefix => Some(format!("[{stage}] {}: {}\n", level.as_str(), message)),
        StageEvent::Log { level, message, .. } => {
            Some(format!("{}: {}\n", level.as_str(), message))
        }
        StageEvent::Finished {
            stage,
            status,
            elapsed_ms,
        } if include_lifecycle && include_stage_prefix => Some(format!(
            "[{stage}] >>> completed {} ({}, {} ms)\n",
            stage,
            format_stage_status_name(*status),
            elapsed_ms
        )),
        StageEvent::Finished {
            stage,
            status,
            elapsed_ms,
        } if include_lifecycle => Some(format!(
            ">>> completed {} ({}, {} ms)\n",
            stage,
            format_stage_status_name(*status),
            elapsed_ms
        )),
        StageEvent::Finished { .. } => None,
    }
}

pub fn render_summary_report(report: &ImplementationReport) -> String {
    let mut out = String::new();
    out.push_str("FDE Implementation Summary\n");
    out.push_str("==========================\n");
    out.push_str(&format!("Design         : {}\n", report.design));
    out.push_str(&format!(
        "Status         : {}\n",
        format_status(report.status)
    ));
    out.push_str(&format!("Seed           : {}\n", report.seed));
    if let Some(elapsed_ms) = report.elapsed_ms {
        out.push_str(&format!(
            "Total runtime  : {}\n",
            format_elapsed_ms(elapsed_ms)
        ));
    }
    if let Some(bitstream_sha256) = report.bitstream_sha256.as_deref() {
        out.push_str(&format!("Bitstream SHA  : {}\n", bitstream_sha256));
    }

    if !report.inputs.is_empty() {
        out.push_str("\nInputs\n------\n");
        for (key, value) in &report.inputs {
            out.push_str(&format!("{:14}: {}\n", key, value));
        }
    }

    if !report.resources.is_empty() {
        out.push_str("\nResources\n---------\n");
        for (key, value) in &report.resources {
            out.push_str(&format!("{:14}: {}\n", key, value));
        }
    }

    out.push_str("\nStage Runtime\n-------------\n");
    for stage in &report.stages {
        let elapsed = stage
            .elapsed_ms
            .map(format_elapsed_ms)
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!("{:14}: {}\n", stage.stage, elapsed));
    }

    out.push_str("\nQoR Summary\n-----------\n");
    if let Some(stage) = report.stages.iter().find(|stage| stage.stage == "map") {
        push_metric_line(&mut out, "Mapped cells", stage.metrics.get("cell_count"));
        push_metric_line(&mut out, "Mapped nets", stage.metrics.get("net_count"));
    }
    if let Some(stage) = report.stages.iter().find(|stage| stage.stage == "pack") {
        push_metric_line(&mut out, "Clusters", stage.metrics.get("cluster_count"));
        push_metric_line(
            &mut out,
            "Cluster cap",
            stage.metrics.get("cluster_capacity"),
        );
    }
    if let Some(stage) = report.stages.iter().find(|stage| stage.stage == "place") {
        push_metric_line(&mut out, "Place cost", stage.metrics.get("final_cost"));
    }
    if let Some(stage) = report.stages.iter().find(|stage| stage.stage == "route") {
        push_metric_line(
            &mut out,
            "Route pips",
            stage.metrics.get("physical_pip_count"),
        );
        push_metric_line(
            &mut out,
            "Route sites",
            stage.metrics.get("routed_site_count"),
        );
        push_metric_line(
            &mut out,
            "Device nets",
            stage.metrics.get("device_net_count"),
        );
    }
    if let Some(timing) = report.timing.as_ref() {
        out.push_str(&format!(
            "{:14}: {:.3} ns\n",
            "Critical path", timing.critical_path_ns
        ));
        out.push_str(&format!("{:14}: {:.2} MHz\n", "Fmax", timing.fmax_mhz));
    }

    out
}

pub fn render_detailed_log(report: &ImplementationReport) -> String {
    let mut out = String::new();
    out.push_str("FDE Run Log\n");
    out.push_str("===========\n");
    out.push_str(&format!("Schema version : {}\n", report.schema_version));
    out.push_str(&format!("Flow           : {}\n", report.flow));
    out.push_str(&format!("Design         : {}\n", report.design));
    out.push_str(&format!(
        "Status         : {}\n",
        format_status(report.status)
    ));
    out.push_str(&format!("Seed           : {}\n", report.seed));
    if let Some(elapsed_ms) = report.elapsed_ms {
        out.push_str(&format!(
            "Total runtime  : {}\n",
            format_elapsed_ms(elapsed_ms)
        ));
    }
    out.push('\n');

    push_mapping_section(&mut out, "Inputs", &report.inputs);
    push_mapping_section(&mut out, "Resources", &report.resources);
    push_mapping_section(&mut out, "Artifacts", &report.artifacts);

    out.push_str("Stages\n------\n");
    for stage in &report.stages {
        let elapsed = stage
            .elapsed_ms
            .map(format_elapsed_ms)
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "{} [{}] elapsed={}\n",
            stage.stage,
            format_status(stage.status),
            elapsed
        ));
        if !stage.metrics.is_empty() {
            out.push_str("  Metrics:\n");
            for (key, value) in &stage.metrics {
                out.push_str(&format!("    - {} = {}\n", key, format_metric_value(value)));
            }
        }
        if !stage.artifacts.is_empty() {
            out.push_str("  Artifacts:\n");
            for (key, value) in &stage.artifacts {
                out.push_str(&format!("    - {} = {}\n", key, value));
            }
        }
        if !stage.warnings.is_empty() {
            out.push_str("  Warnings:\n");
            for warning in &stage.warnings {
                out.push_str(&format!("    - {}\n", warning));
            }
        }
        if !stage.messages.is_empty() {
            out.push_str("  Messages:\n");
            for message in &stage.messages {
                out.push_str(&format!("    - {}\n", message));
            }
        }
        out.push('\n');
    }

    if let Some(timing) = report.timing.as_ref() {
        out.push_str("Timing\n------\n");
        out.push_str(&format!(
            "critical_path_ns = {:.6}\n",
            timing.critical_path_ns
        ));
        out.push_str(&format!("fmax_mhz         = {:.6}\n", timing.fmax_mhz));
        if !timing.top_paths.is_empty() {
            out.push_str("top_paths:\n");
            for (index, path) in timing.top_paths.iter().enumerate() {
                out.push_str(&format!(
                    "  {}. {:?} endpoint={} delay_ns={:.6}\n",
                    index + 1,
                    path.category,
                    path.endpoint,
                    path.delay_ns
                ));
            }
        }
    }

    out
}

fn push_mapping_section(out: &mut String, title: &str, values: &BTreeMap<String, String>) {
    if values.is_empty() {
        return;
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(&"-".repeat(title.len()));
    out.push('\n');
    for (key, value) in values {
        out.push_str(&format!("{:14}: {}\n", key, value));
    }
    out.push('\n');
}

fn push_metric_line(out: &mut String, label: &str, value: Option<&Value>) {
    if let Some(value) = value {
        out.push_str(&format!("{:14}: {}\n", label, format_metric_value(value)));
    }
}

pub fn format_stage_status_name(status: ReportStatus) -> &'static str {
    match status {
        ReportStatus::Success => "success",
        ReportStatus::Failed => "failed",
        ReportStatus::Skipped => "skipped",
    }
}

fn format_status(status: ReportStatus) -> &'static str {
    match status {
        ReportStatus::Success => "SUCCESS",
        ReportStatus::Failed => "FAILED",
        ReportStatus::Skipped => "SKIPPED",
    }
}

fn format_elapsed_ms(elapsed_ms: u64) -> String {
    if elapsed_ms == 0 {
        return "<1 ms".to_string();
    }
    if elapsed_ms >= 1_000 {
        format!("{:.3} s", elapsed_ms as f64 / 1_000.0)
    } else {
        format!("{elapsed_ms} ms")
    }
}

fn format_metric_pairs(metrics: &ReportMetrics) -> String {
    metrics
        .iter()
        .map(|(key, value)| format!("{key}={}", format_metric_value(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_metric_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => match (value.as_u64(), value.as_i64(), value.as_f64()) {
            (Some(value), _, _) => value.to_string(),
            (_, Some(value), _) => value.to_string(),
            (_, _, Some(value)) => {
                let rounded = format!("{value:.3}");
                rounded
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            }
            _ => value.to_string(),
        },
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_stage_event_line_omits_lifecycle_when_disabled() {
        assert_eq!(
            format_stage_event_line(&StageEvent::Started { stage: "route" }, false, true),
            None
        );
        assert_eq!(
            format_stage_event_line(
                &StageEvent::Finished {
                    stage: "route",
                    status: ReportStatus::Success,
                    elapsed_ms: 42,
                },
                false,
                true,
            ),
            None
        );
    }

    #[test]
    fn format_stage_event_line_formats_cli_progress_with_stage_prefix() {
        assert_eq!(
            format_stage_event_line(
                &StageEvent::Log {
                    stage: "route",
                    level: StageLogLevel::Progress,
                    message: "routed 12/34 nets".to_string(),
                },
                true,
                true,
            ),
            Some("[route] progress: routed 12/34 nets\n".to_string())
        );
    }

    #[test]
    fn line_stage_reporter_runtime_only_emits_log_without_prefix_or_lifecycle() {
        let mut lines = Vec::new();
        let mut logger = |line: String| lines.push(line);
        let mut reporter = LineStageReporter::runtime_only(&mut logger);

        reporter.on_stage_event(StageEvent::Started { stage: "pack" });
        reporter.on_stage_event(StageEvent::Log {
            stage: "pack",
            level: StageLogLevel::Info,
            message: "clustered 16 cells".to_string(),
        });
        reporter.on_stage_event(StageEvent::Finished {
            stage: "pack",
            status: ReportStatus::Success,
            elapsed_ms: 7,
        });

        assert_eq!(lines, vec!["info: clustered 16 cells\n".to_string()]);
    }

    #[test]
    fn line_stage_reporter_cli_emits_lifecycle_and_prefixed_logs() {
        let mut lines = Vec::new();
        let mut logger = |line: String| lines.push(line);
        let mut reporter = LineStageReporter::cli(&mut logger);

        reporter.on_stage_event(StageEvent::Started { stage: "map" });
        reporter.on_stage_event(StageEvent::Log {
            stage: "map",
            level: StageLogLevel::Info,
            message: "loaded 3 cells".to_string(),
        });
        reporter.on_stage_event(StageEvent::Finished {
            stage: "map",
            status: ReportStatus::Success,
            elapsed_ms: 5,
        });

        assert_eq!(
            lines,
            vec![
                "[map] >>> starting map\n".to_string(),
                "[map] info: loaded 3 cells\n".to_string(),
                "[map] >>> completed map (success, 5 ms)\n".to_string(),
            ]
        );
    }
}
