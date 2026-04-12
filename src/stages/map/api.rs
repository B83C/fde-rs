use super::{rewrite::rewrite_design, verilog::export_structural_verilog};
use crate::{
    ir::Design,
    report::{StageOutput, StageReport, StageReporter, emit_stage_info},
};
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MapOptions {
    pub lut_size: usize,
    pub cell_library: Option<PathBuf>,
    pub emit_structural_verilog: bool,
}

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            lut_size: 4,
            cell_library: None,
            emit_structural_verilog: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MapArtifact {
    pub design: Design,
    pub structural_verilog: Option<String>,
}

pub fn run(design: Design, options: &MapOptions) -> Result<StageOutput<MapArtifact>> {
    run_internal(design, options, None)
}

pub fn run_with_reporter(
    design: Design,
    options: &MapOptions,
    reporter: &mut dyn StageReporter,
) -> Result<StageOutput<MapArtifact>> {
    run_internal(design, options, Some(reporter))
}

fn run_internal(
    mut design: Design,
    options: &MapOptions,
    mut reporter: Option<&mut dyn StageReporter>,
) -> Result<StageOutput<MapArtifact>> {
    design.stage = "mapped".to_string();
    design.metadata.lut_size = options.lut_size;
    if design.metadata.source_format.is_empty() {
        design.metadata.source_format = "ir".to_string();
    }
    emit_stage_info(
        &mut reporter,
        "map",
        format!(
            "mapping design '{}' with LUT size {}",
            design.name, options.lut_size
        ),
    );
    if let Some(cell_library) = &options.cell_library {
        design.note_once(format!(
            "Mapping referenced cell library {}",
            cell_library.display()
        ));
        emit_stage_info(
            &mut reporter,
            "map",
            format!("using cell library {}", cell_library.display()),
        );
    }

    let summary = rewrite_design(&mut design, options)?;
    emit_stage_info(
        &mut reporter,
        "map",
        format!(
            "rewrite complete: {} cells, {} nets, {} normalized LUTs, {} block RAMs",
            design.cells.len(),
            design.nets.len(),
            summary.normalized_luts,
            summary.normalized_block_rams
        ),
    );

    let structural_verilog = options
        .emit_structural_verilog
        .then(|| export_structural_verilog(&design));
    if structural_verilog.is_some() {
        emit_stage_info(&mut reporter, "map", "emitted structural Verilog artifact");
    }

    let mut report = StageReport::new("map");
    report.metric("cell_count", design.cells.len());
    report.metric("net_count", design.nets.len());
    report.metric("lut_size", options.lut_size);
    report.metric("normalized_lut_count", summary.normalized_luts);
    report.metric("normalized_block_ram_count", summary.normalized_block_rams);
    report.metric("lowered_constant_count", summary.lowered_constants);
    report.metric("buffered_ff_input_count", summary.buffered_ff_inputs);
    report.push(format!(
        "Mapped {} cells and {} nets.",
        design.cells.len(),
        design.nets.len()
    ));
    if summary.normalized_luts > 0 {
        report.push(format!(
            "Normalized repeated LUT inputs in {} cells.",
            summary.normalized_luts
        ));
    }
    if summary.normalized_block_rams > 0 {
        report.push(format!(
            "Canonicalized {} block RAM cells into BLOCKRAM_1/BLOCKRAM_2 forms.",
            summary.normalized_block_rams
        ));
    }
    if summary.lowered_constants > 0 {
        report.push(format!(
            "Lowered {} constant source cells into LUT-backed drivers.",
            summary.lowered_constants
        ));
    }
    if summary.buffered_ff_inputs > 0 {
        report.push(format!(
            "Inserted {} LUT buffers on non-LUT FF data inputs.",
            summary.buffered_ff_inputs
        ));
    }

    Ok(StageOutput {
        value: MapArtifact {
            design,
            structural_verilog,
        },
        report,
    })
}
