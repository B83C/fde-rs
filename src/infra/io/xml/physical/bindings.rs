use crate::ir::{
    AssignedClusterCellKind, Cell, ClusterId, Design, DesignIndex, assign_cluster_slice_cells,
};
use std::collections::{BTreeMap, BTreeSet};

use super::super::writer::{
    SLICE_DEFAULT_CONFIGS, SliceCellBinding, SliceCellKind, default_config_map, ordered_configs,
    packed_lut_function_name,
};

pub(super) fn assign_cluster_cells(
    design: &Design,
    index: &DesignIndex<'_>,
    cluster_id: ClusterId,
) -> BTreeMap<String, SliceCellBinding> {
    assign_cluster_slice_cells(design, index, cluster_id)
        .into_iter()
        .map(|assignment| {
            let cell = index.cell(design, assignment.cell_id);
            (
                cell.name.clone(),
                SliceCellBinding {
                    slot: assignment.slot,
                    kind: match assignment.kind {
                        AssignedClusterCellKind::Lut => SliceCellKind::Lut,
                        AssignedClusterCellKind::Sequential => SliceCellKind::Sequential,
                        AssignedClusterCellKind::Other => SliceCellKind::Other,
                    },
                },
            )
        })
        .collect()
}

pub(super) fn build_slice_configs(
    design: &Design,
    cells: &BTreeMap<String, SliceCellBinding>,
) -> Vec<(String, String)> {
    let mut configs = default_config_map(SLICE_DEFAULT_CONFIGS);
    for (cell_name, binding) in cells {
        let Some(cell) = design
            .cells
            .iter()
            .find(|candidate| candidate.name == *cell_name)
        else {
            continue;
        };
        if binding.kind == SliceCellKind::Lut
            && let Some(function) = packed_lut_function_name(cell)
        {
            let cfg_name = if binding.slot == 0 { "F" } else { "G" };
            let mux_name = if binding.slot == 0 { "FXMUX" } else { "GYMUX" };
            configs.insert(cfg_name.to_string(), function);
            configs.insert(mux_name.to_string(), cfg_name.to_string());
            let used_name = if binding.slot == 0 { "XUSED" } else { "YUSED" };
            let used_value = if lut_has_routed_sink(design, cell, cells, binding.slot) {
                "0"
            } else {
                "#OFF"
            };
            configs.insert(used_name.to_string(), used_value.to_string());
        }
        if binding.kind == SliceCellKind::Sequential {
            let ff_name = if binding.slot == 0 { "FFX" } else { "FFY" };
            let init_name = if binding.slot == 0 { "INITX" } else { "INITY" };
            let d_name = if binding.slot == 0 { "DXMUX" } else { "DYMUX" };
            let b_name = if binding.slot == 0 { "BXMUX" } else { "BYMUX" };
            let xused_name = if binding.slot == 0 { "XUSED" } else { "YUSED" };
            configs.insert(ff_name.to_string(), "#FF".to_string());
            configs.insert(init_name.to_string(), "LOW".to_string());
            configs.insert("SYNC_ATTR".to_string(), "ASYNC".to_string());
            configs.insert("CKINV".to_string(), "1".to_string());
            if ff_uses_clock_enable(cell) {
                configs.insert("CEMUX".to_string(), "CE".to_string());
            }
            if ff_uses_site_bypass(design, cell, cells, binding.slot) {
                configs.insert(d_name.to_string(), "0".to_string());
                configs.insert(
                    b_name.to_string(),
                    if binding.slot == 0 { "BX" } else { "BY" }.to_string(),
                );
            } else {
                configs.insert(d_name.to_string(), "1".to_string());
            }
            configs
                .entry(xused_name.to_string())
                .or_insert_with(|| "#OFF".to_string());
        }
    }
    ordered_configs(SLICE_DEFAULT_CONFIGS, configs)
}

fn ff_uses_clock_enable(cell: &Cell) -> bool {
    cell.register_clock_enable_net().is_some()
}

fn ff_uses_site_bypass(
    design: &Design,
    ff: &Cell,
    bindings: &BTreeMap<String, SliceCellBinding>,
    slot: usize,
) -> bool {
    let Some(d_net) = ff
        .inputs
        .iter()
        .find(|pin| pin.port.eq_ignore_ascii_case("D"))
        .map(|pin| pin.net.as_str())
    else {
        return false;
    };
    let Some(net) = design.nets.iter().find(|net| net.name == d_net) else {
        return false;
    };
    let Some(driver) = net.driver.as_ref() else {
        return true;
    };
    let crate::domain::EndpointKind::Cell = driver.kind else {
        return true;
    };
    let Some(driver_cell) = design.cells.iter().find(|cell| cell.name == driver.name) else {
        return true;
    };
    let Some(binding) = bindings.get(driver_cell.name.as_str()) else {
        return true;
    };
    !(driver_cell.is_lut() && binding.slot.min(1) == slot.min(1))
}

fn lut_has_routed_sink(
    design: &Design,
    lut: &Cell,
    bindings: &BTreeMap<String, SliceCellBinding>,
    slot: usize,
) -> bool {
    let output_nets = lut
        .outputs
        .iter()
        .map(|pin| pin.net.as_str())
        .collect::<BTreeSet<_>>();
    design.nets.iter().any(|net| {
        output_nets.contains(net.name.as_str())
            && net.sinks.iter().any(|sink| {
                let Some(sink_cell) = design.cells.iter().find(|cell| cell.name == sink.name)
                else {
                    return true;
                };
                let Some(binding) = bindings.get(sink_cell.name.as_str()) else {
                    return true;
                };
                !(sink_cell.is_sequential()
                    && sink.pin.eq_ignore_ascii_case("D")
                    && binding.slot.min(1) == slot.min(1))
            })
    })
}
