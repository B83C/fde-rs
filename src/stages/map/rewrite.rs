use super::api::MapOptions;
use super::lut::{
    all_ones_truth_table, all_zeros_truth_table, canonicalize_lut_init, default_lut_mask,
    format_lut_init_hex, infer_lut_width, parse_lut_init_value,
};
use crate::{
    domain::{CellKind, ConstantKind, PrimitiveKind, pin_map_property_name},
    ir::{Cell, CellId, Design, Endpoint, EndpointKind},
    normalize::prune_disconnected_nets,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RewriteSummary {
    pub(super) normalized_luts: usize,
    pub(super) lowered_constants: usize,
    pub(super) buffered_ff_inputs: usize,
}

pub(super) fn rewrite_design(design: &mut Design, options: &MapOptions) -> RewriteSummary {
    let lut_inits_are_decimal = design.metadata.source_format.eq_ignore_ascii_case("edif");

    for cell in &mut design.cells {
        if cell.is_lut() {
            canonicalize_lut_init(cell, lut_inits_are_decimal);
        }
        if cell.is_lut() && cell.property("lut_init").is_none() {
            let width = infer_lut_width(&cell.type_name).max(1);
            cell.set_property("lut_init", default_lut_mask(width));
        }
        if matches!(cell.primitive_kind(), PrimitiveKind::Generic) {
            let input_count = cell.inputs.len().clamp(1, options.lut_size.max(1));
            cell.kind = CellKind::Lut;
            cell.type_name = format!("LUT{}", input_count.max(2));
            canonicalize_lut_init(cell, lut_inits_are_decimal);
            if cell.property("lut_init").is_none() {
                cell.set_property("lut_init", default_lut_mask(input_count));
            }
        }
    }

    let normalized_luts = normalize_repeated_lut_inputs(design);
    let lowered_constants = lower_constant_sources(design, options.lut_size.max(1));
    let buffered_ff_inputs = buffer_non_lut_ff_inputs(design);
    prune_disconnected_nets(design);

    RewriteSummary {
        normalized_luts,
        lowered_constants,
        buffered_ff_inputs,
    }
}

pub(super) fn normalize_repeated_lut_inputs(design: &mut Design) -> usize {
    let mut normalized = 0usize;

    for cell in &mut design.cells {
        if !cell.is_lut() {
            continue;
        }
        if normalize_lut_cell_inputs(cell) {
            normalized += 1;
        }
    }

    if normalized > 0 {
        sync_cell_input_sinks(design);
    }

    normalized
}

fn normalize_lut_cell_inputs(cell: &mut Cell) -> bool {
    let primitive = cell.primitive_kind();
    let old_width = infer_lut_width(&cell.type_name).max(1);
    let Some(init) = cell.property("lut_init").map(str::to_owned) else {
        return false;
    };
    let Some(old_value) = parse_lut_init_value(&init, false) else {
        return false;
    };

    let mut old_to_new = vec![None; old_width];
    let mut new_to_old = Vec::<Vec<usize>>::new();
    let mut unique_nets = Vec::<String>::new();
    let mut unique_index = BTreeMap::<String, usize>::new();

    let mut indexed_inputs = cell
        .inputs
        .iter()
        .filter_map(|pin| {
            primitive
                .lut_input_index(&pin.port)
                .filter(|index| *index < old_width)
                .map(|index| (index, pin.net.clone()))
        })
        .collect::<Vec<_>>();
    indexed_inputs.sort_by_key(|(index, _)| *index);

    for (old_index, net) in indexed_inputs {
        let new_index = if let Some(existing) = unique_index.get(&net) {
            *existing
        } else {
            let index = unique_nets.len();
            unique_index.insert(net.clone(), index);
            unique_nets.push(net);
            new_to_old.push(Vec::new());
            index
        };
        old_to_new[old_index] = Some(new_index);
        new_to_old[new_index].push(old_index);
    }

    let new_width = unique_nets.len().max(1);
    let has_duplicate_inputs = unique_nets.len() < cell.inputs.len();
    if !has_duplicate_inputs && new_width == old_width {
        return false;
    }

    let new_truth_table_bits = 1usize.checked_shl(new_width.min(7) as u32).unwrap_or(128);
    let mut new_value = 0u128;
    for new_address in 0..new_truth_table_bits {
        let mut old_address = 0usize;
        for (old_index, maybe_new_index) in old_to_new.iter().enumerate() {
            let bit = maybe_new_index
                .map(|new_index| (new_address >> new_index) & 1)
                .unwrap_or(0);
            old_address |= bit << old_index;
        }
        if ((old_value >> old_address) & 1) != 0 {
            new_value |= 1u128 << new_address;
        }
    }

    cell.inputs = unique_nets
        .into_iter()
        .enumerate()
        .map(|(index, net)| crate::ir::CellPin::new(format!("ADR{index}"), net))
        .collect();
    cell.type_name = format!("LUT{new_width}");
    cell.set_property("lut_init", format_lut_init_hex(new_value, new_width));
    for (new_index, old_indices) in new_to_old.into_iter().enumerate() {
        if old_indices.len() == 1 && old_indices[0] == new_index {
            continue;
        }
        let value = old_indices
            .into_iter()
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        cell.set_property(pin_map_property_name(new_index), value);
    }

    true
}

fn sync_cell_input_sinks(design: &mut Design) {
    let mut cell_sinks_by_net = BTreeMap::<String, Vec<Endpoint>>::new();
    for cell in &design.cells {
        for input in &cell.inputs {
            cell_sinks_by_net
                .entry(input.net.clone())
                .or_default()
                .push(Endpoint::new(
                    EndpointKind::Cell,
                    cell.name.clone(),
                    input.port.clone(),
                ));
        }
    }

    for net in &mut design.nets {
        let mut sinks = net
            .sinks
            .iter()
            .filter(|endpoint| endpoint.kind != EndpointKind::Cell)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(cell_sinks) = cell_sinks_by_net.get(&net.name) {
            sinks.extend(cell_sinks.iter().cloned());
        }
        net.sinks = sinks;
    }
}

fn lower_constant_sources(design: &mut Design, lut_size: usize) -> usize {
    let lut_size = lut_size.max(1);
    let mut lowered = BTreeSet::new();

    for (cell_index, cell) in design.cells.iter_mut().enumerate() {
        let Some(init) = constant_lut_init(cell, lut_size) else {
            continue;
        };
        if cell.outputs.is_empty() {
            continue;
        }
        cell.kind = CellKind::Lut;
        cell.type_name = format!("LUT{lut_size}");
        cell.inputs.clear();
        for output in &mut cell.outputs {
            output.port = "O".to_string();
        }
        cell.set_property("lut_init", init);
        lowered.insert(CellId::new(cell_index));
    }

    if lowered.is_empty() {
        return 0;
    }

    let lowered_net_drivers = {
        let index = design.index();
        design
            .nets
            .iter()
            .map(|net| {
                net.driver
                    .as_ref()
                    .and_then(|driver| index.cell_for_endpoint(driver))
                    .is_some_and(|cell_id| lowered.contains(&cell_id))
            })
            .collect::<Vec<_>>()
    };

    for (net, is_lowered_driver) in design.nets.iter_mut().zip(lowered_net_drivers) {
        if is_lowered_driver && let Some(driver) = &mut net.driver {
            driver.pin = "O".to_string();
        }
    }

    lowered.len()
}

fn buffer_non_lut_ff_inputs(design: &mut Design) -> usize {
    let drivers = {
        let index = design.index();
        design
            .cells
            .iter()
            .enumerate()
            .filter_map(|(cell_index, cell)| {
                let d_pin = cell
                    .is_sequential()
                    .then(|| {
                        cell.inputs
                            .iter()
                            .find(|pin| cell.primitive_kind().is_register_data_pin(&pin.port))
                    })
                    .flatten()?;
                let net_id = index.net_id(&d_pin.net)?;
                let driver = index.net(design, net_id).driver.as_ref()?;
                let driver_is_lut = index
                    .cell_for_endpoint(driver)
                    .is_some_and(|driver_cell_id| index.cell(design, driver_cell_id).is_lut());
                (!driver_is_lut).then(|| {
                    (
                        CellId::new(cell_index),
                        d_pin.port.clone(),
                        d_pin.net.clone(),
                    )
                })
            })
            .collect::<Vec<_>>()
    };

    if drivers.is_empty() {
        return 0;
    }

    let mut used_names = design
        .cells
        .iter()
        .map(|cell| cell.name.clone())
        .chain(design.nets.iter().map(|net| net.name.clone()))
        .collect::<BTreeSet<_>>();

    for (ff_id, d_port, source_net) in &drivers {
        let ff_name = design.cells[ff_id.index()].name.clone();
        let lut_name = unique_name(&mut used_names, format!("{ff_name}__d_buf_lut"));
        let buffered_net = unique_name(&mut used_names, format!("{ff_name}__d_buf_net"));

        let mut buffer = Cell::lut(lut_name.clone(), "LUT1")
            .with_input("ADR0", source_net.clone())
            .with_output("O", buffered_net.clone());
        buffer.set_property("lut_init", format_lut_init_hex(0b10, 1));
        design.cells.push(buffer);
        if let Some(cell) = design.cells.get_mut(ff_id.index())
            && let Some(d_pin) = cell.inputs.iter_mut().find(|pin| pin.port == *d_port)
        {
            d_pin.net = buffered_net.clone();
        }
        design
            .nets
            .push(crate::ir::Net::new(buffered_net).with_driver(Endpoint::cell(lut_name, "O")));
    }

    sync_cell_input_sinks(design);
    drivers.len()
}

fn unique_name(used: &mut BTreeSet<String>, base: String) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut suffix = 0usize;
    loop {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn constant_lut_init(cell: &Cell, lut_size: usize) -> Option<String> {
    match cell.constant_kind() {
        Some(ConstantKind::Zero) => Some(all_zeros_truth_table(lut_size)),
        Some(ConstantKind::One) => Some(all_ones_truth_table(lut_size)),
        Some(ConstantKind::Unknown) | None => None,
    }
}
