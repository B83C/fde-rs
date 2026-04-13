use crate::ir::{
    AssignedClusterCellKind, Cell, ClusterId, Design, DesignIndex, assign_cluster_slice_cells,
};
use std::collections::{BTreeMap, BTreeSet};

use super::super::writer::{
    SLICE_DEFAULT_CONFIGS, SliceCellBinding, SliceCellKind, default_config_map, ordered_configs,
    physical_lut_function_name,
};
use crate::domain::{SequentialInitValue, SliceSequentialConfigKey, SliceSlot};

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
        let Some(slot) = SliceSlot::from_index(binding.slot.min(1)) else {
            continue;
        };
        let Some(cell) = design
            .cells
            .iter()
            .find(|candidate| candidate.name == *cell_name)
        else {
            continue;
        };
        if binding.kind == SliceCellKind::Lut
            && let Some(function) = physical_lut_function_name(cell)
        {
            let cfg_name = slot.lut_config_name();
            configs.insert(cfg_name.to_string(), function);
            configs.insert(slot.lut_mux_config_name().to_string(), cfg_name.to_string());
            let used_value = if lut_has_routed_sink(design, cell, cells, binding.slot) {
                "0"
            } else {
                "#OFF"
            };
            configs.insert(
                slot.lut_used_config_name().to_string(),
                used_value.to_string(),
            );
        }
        if binding.kind == SliceCellKind::Sequential {
            configs.insert(slot.ff_config_name().to_string(), "#FF".to_string());
            configs.insert(
                slot.init_config_name().to_string(),
                cell.register_init_value()
                    .unwrap_or(SequentialInitValue::Low)
                    .as_config_value()
                    .to_string(),
            );
            configs.insert(
                SliceSequentialConfigKey::SyncAttr.as_str().to_string(),
                cell.property("SYNC_ATTR").unwrap_or("ASYNC").to_string(),
            );
            configs.insert(
                SliceSequentialConfigKey::ClockInvert.as_str().to_string(),
                cell.property("CKINV")
                    .unwrap_or(if cell.register_clock_is_inverted() {
                        "1"
                    } else {
                        "#OFF"
                    })
                    .to_string(),
            );
            if let Some(value) = cell.property("CEMUX").filter(|value| *value != "#OFF") {
                configs.insert(
                    SliceSequentialConfigKey::ClockEnableMux
                        .as_str()
                        .to_string(),
                    value.to_string(),
                );
            } else if ff_uses_clock_enable(cell) {
                configs.insert(
                    SliceSequentialConfigKey::ClockEnableMux
                        .as_str()
                        .to_string(),
                    "CE".to_string(),
                );
            }
            if let Some(value) = cell.property("SRMUX").filter(|value| *value != "#OFF") {
                configs.insert(
                    SliceSequentialConfigKey::SetResetMux.as_str().to_string(),
                    value.to_string(),
                );
            }
            if let Some(value) = cell.property("SRFFMUX").filter(|value| *value != "#OFF") {
                configs.insert(
                    SliceSequentialConfigKey::SetResetFfMux.as_str().to_string(),
                    value.to_string(),
                );
            }
            if ff_uses_site_bypass(design, cell, cells, binding.slot) {
                configs.insert(slot.data_mux_config_name().to_string(), "0".to_string());
                configs.insert(
                    slot.bypass_mux_config_name().to_string(),
                    slot.bypass_function_name().to_string(),
                );
            } else {
                configs.insert(slot.data_mux_config_name().to_string(), "1".to_string());
            }
            configs
                .entry(slot.lut_used_config_name().to_string())
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

#[cfg(test)]
mod tests {
    use super::{SliceCellBinding, SliceCellKind, build_slice_configs};
    use crate::infra::io::xml::lut_expr::PHYSICAL_LUT_FUNCTION_PROPERTY;
    use crate::ir::{Cell, CellKind, Design};
    use std::collections::BTreeMap;

    #[test]
    fn slice_configs_preserve_high_ff_init_property() {
        let mut ff = Cell::new("ff0", CellKind::Ff, "DFFHQ");
        ff.set_property("init", "1");
        let design = Design {
            cells: vec![ff],
            ..Design::default()
        };
        let bindings = BTreeMap::from([(
            "ff0".to_string(),
            SliceCellBinding {
                slot: 0,
                kind: SliceCellKind::Sequential,
            },
        )]);

        let configs = build_slice_configs(&design, &bindings);

        assert!(
            configs
                .iter()
                .any(|(name, value)| name == "INITX" && value == "HIGH")
        );
    }

    #[test]
    fn slice_configs_preserve_imported_ff_site_control_muxes() {
        let mut ff = Cell::new("ff0", CellKind::Ff, "DFFHQ");
        ff.set_property("SYNC_ATTR", "ASYNC");
        ff.set_property("CKINV", "1");
        ff.set_property("SRMUX", "SR_B");
        ff.set_property("SRFFMUX", "0");
        let design = Design {
            cells: vec![ff],
            ..Design::default()
        };
        let bindings = BTreeMap::from([(
            "ff0".to_string(),
            SliceCellBinding {
                slot: 0,
                kind: SliceCellKind::Sequential,
            },
        )]);

        let configs = build_slice_configs(&design, &bindings);

        assert!(
            configs
                .iter()
                .any(|(name, value)| name == "SRMUX" && value == "SR_B")
        );
        assert!(
            configs
                .iter()
                .any(|(name, value)| name == "SRFFMUX" && value == "0")
        );
    }

    #[test]
    fn slice_configs_preserve_imported_cpp_constant_lut_function_spelling() {
        let mut lut = Cell::new("lut0", CellKind::Lut, "LUT4");
        lut.set_property("lut_init", "0xFFFF");
        lut.set_property(PHYSICAL_LUT_FUNCTION_PROPERTY, "#LUT:D=1");
        let design = Design {
            cells: vec![lut],
            ..Design::default()
        };
        let bindings = BTreeMap::from([(
            "lut0".to_string(),
            SliceCellBinding {
                slot: 0,
                kind: SliceCellKind::Lut,
            },
        )]);

        let configs = build_slice_configs(&design, &bindings);

        assert!(
            configs
                .iter()
                .any(|(name, value)| name == "F" && value == "#LUT:D=1")
        );
    }
}
