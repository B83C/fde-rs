use super::{circuit::BitgenCircuit, generator::generate_bitstream};
use crate::{
    cil::Cil,
    ir::{BitstreamImage, Design},
    report::{StageOutput, StageReporter, emit_stage_info},
    route::DeviceRouteImage,
};
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct BitgenOptions {
    pub arch_name: Option<String>,
    pub arch_path: Option<PathBuf>,
    pub cil_path: Option<PathBuf>,
    pub cil: Option<Cil>,
    pub device_design: Option<super::DeviceDesign>,
    pub route_image: Option<DeviceRouteImage>,
}

pub fn run(design: Design, options: &BitgenOptions) -> Result<StageOutput<BitstreamImage>> {
    run_internal(design, options, None)
}

pub fn run_with_reporter(
    design: Design,
    options: &BitgenOptions,
    reporter: &mut dyn StageReporter,
) -> Result<StageOutput<BitstreamImage>> {
    run_internal(design, options, Some(reporter))
}

fn run_internal(
    design: Design,
    options: &BitgenOptions,
    mut reporter: Option<&mut dyn StageReporter>,
) -> Result<StageOutput<BitstreamImage>> {
    let mut design = design;
    emit_stage_info(
        &mut reporter,
        "bitgen",
        format!(
            "generating bitstream for design '{}' ({} nets, {} cells)",
            design.name,
            design.nets.len(),
            design.cells.len()
        ),
    );
    design.infer_slice_bindings_from_route_pips();
    emit_stage_info(
        &mut reporter,
        "bitgen",
        "inferred slice bindings from routed design",
    );
    let circuit = BitgenCircuit::from_design(&design);
    emit_stage_info(
        &mut reporter,
        "bitgen",
        format!(
            "materialized bitgen circuit with {} clusters and {} nets",
            circuit.clusters.len(),
            circuit.nets.len()
        ),
    );
    generate_bitstream(&circuit, options)
}
