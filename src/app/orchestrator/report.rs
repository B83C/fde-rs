use crate::{
    ir::TimingSummary,
    report::{
        ImplementationReport, ReportStatus, StageReport, render_detailed_log, render_summary_report,
    },
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct FlowArtifacts {
    pub(crate) map: PathBuf,
    pub(crate) pack: PathBuf,
    pub(crate) place: PathBuf,
    pub(crate) route: PathBuf,
    pub(crate) device: Option<PathBuf>,
    pub(crate) sta: PathBuf,
    pub(crate) sta_report: PathBuf,
    pub(crate) bitstream: PathBuf,
    pub(crate) bitstream_sidecar: Option<PathBuf>,
    pub(crate) report: PathBuf,
    pub(crate) summary: PathBuf,
    pub(crate) log: PathBuf,
}

impl FlowArtifacts {
    pub(crate) fn modern(out_dir: &Path, emit_sidecar: bool) -> Self {
        Self {
            map: out_dir.join("01-mapped.xml"),
            pack: out_dir.join("02-packed.xml"),
            place: out_dir.join("03-placed.xml"),
            route: out_dir.join("04-routed.xml"),
            device: Some(out_dir.join("04-device.json")),
            sta: out_dir.join("05-timed.xml"),
            sta_report: out_dir.join("05-timing.rpt"),
            bitstream: out_dir.join("06-output.bit"),
            bitstream_sidecar: emit_sidecar.then(|| out_dir.join("06-output.sidecar.txt")),
            report: out_dir.join("report.json"),
            summary: out_dir.join("summary.rpt"),
            log: out_dir.join("run.log"),
        }
    }

    pub(crate) fn artifact_map(&self) -> BTreeMap<String, String> {
        let mut artifacts = BTreeMap::new();
        artifacts.insert("map".to_string(), self.map.display().to_string());
        artifacts.insert("pack".to_string(), self.pack.display().to_string());
        artifacts.insert("place".to_string(), self.place.display().to_string());
        artifacts.insert("route".to_string(), self.route.display().to_string());
        if let Some(device) = self.device.as_ref().filter(|path| path.exists()) {
            artifacts.insert("device".to_string(), device.display().to_string());
        }
        artifacts.insert("sta".to_string(), self.sta.display().to_string());
        artifacts.insert(
            "sta_report".to_string(),
            self.sta_report.display().to_string(),
        );
        artifacts.insert(
            "bitstream".to_string(),
            self.bitstream.display().to_string(),
        );
        if let Some(sidecar) = self.bitstream_sidecar.as_ref().filter(|path| path.exists()) {
            artifacts.insert(
                "bitstream_sidecar".to_string(),
                sidecar.display().to_string(),
            );
        }
        artifacts.insert("report".to_string(), self.report.display().to_string());
        artifacts.insert("summary".to_string(), self.summary.display().to_string());
        artifacts.insert("log".to_string(), self.log.display().to_string());
        artifacts
    }
}

pub(crate) struct ReportContext {
    pub(crate) flow: String,
    pub(crate) design: String,
    pub(crate) out_dir: PathBuf,
    pub(crate) seed: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) inputs: BTreeMap<String, String>,
    pub(crate) resources: BTreeMap<String, String>,
}

pub(crate) fn build_report(
    context: ReportContext,
    artifacts: &FlowArtifacts,
    stages: Vec<StageReport>,
    timing: Option<TimingSummary>,
    bitstream_sha256: Option<String>,
) -> ImplementationReport {
    ImplementationReport {
        schema_version: 2,
        flow: context.flow,
        design: context.design,
        out_dir: context.out_dir.display().to_string(),
        seed: context.seed,
        status: ReportStatus::Success,
        elapsed_ms: Some(context.elapsed_ms),
        inputs: context.inputs,
        resources: context.resources,
        artifacts: artifacts.artifact_map(),
        stages,
        timing,
        bitstream_sha256,
    }
}

pub(crate) fn write_report(path: &Path, report: &ImplementationReport) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(report)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn write_summary(path: &Path, report: &ImplementationReport) -> Result<()> {
    fs::write(path, render_summary_report(report))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn write_log_with_runtime(
    path: &Path,
    report: &ImplementationReport,
    runtime_log: &str,
) -> Result<()> {
    fs::write(path, render_log_with_runtime(report, runtime_log))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn render_log_with_runtime(report: &ImplementationReport, runtime_log: &str) -> String {
    if runtime_log.trim().is_empty() {
        return render_detailed_log(report);
    }

    let mut out = String::new();
    out.push_str("FDE Runtime Log\n");
    out.push_str("===============\n");
    out.push_str(runtime_log.trim_end());
    out.push_str("\n\n");
    out.push_str(&render_detailed_log(report));
    out
}
