mod block_ram;

use self::block_ram::derive_block_ram_program;
use std::collections::BTreeMap;

use super::types::{
    IobProgram, LutProgram, SequentialProgram, SiteInstance, SiteProgram, SiteProgramKind,
    SliceClockEnableMode, SliceFfDataPath, SliceLutOutputUsage, SliceProgram, SliceSetResetMode,
};
use crate::{
    bitgen::{DeviceCell, DeviceDesign, DeviceEndpoint},
    bitgen::{DeviceDesignIndex, DeviceEndpointRef, literal::normalized_site_lut_truth_table_bits},
    domain::{SequentialInitValue, SiteKind},
};

const LOGIC_SLICE_SITE_INPUTS: usize = 4;

pub(crate) fn derive_site_programs(
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> Vec<SiteProgram> {
    let mut sites = BTreeMap::<(String, String, SiteKind), SiteInstance>::new();
    for cell in &device.cells {
        if !cell.is_sited() {
            continue;
        }
        let key = (
            cell.tile_name.clone(),
            cell.site_name.clone(),
            cell.site_kind,
        );
        let entry = sites.entry(key).or_insert_with(|| SiteInstance {
            tile_name: cell.tile_name.clone(),
            tile_type: cell.tile_type.clone(),
            site_kind: cell.site_kind,
            site_name: cell.site_name.clone(),
            x: cell.x,
            y: cell.y,
            z: cell.z,
            cells: Vec::new(),
        });
        entry.cells.push(cell.clone());
    }

    sites
        .into_values()
        .filter_map(|site| derive_site_program(&site, device, index))
        .collect()
}

fn derive_site_program(
    site: &SiteInstance,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> Option<SiteProgram> {
    let kind = match site.site_kind {
        SiteKind::LogicSlice => {
            SiteProgramKind::LogicSlice(derive_slice_program(site, device, index))
        }
        SiteKind::BlockRam => SiteProgramKind::BlockRam(
            site.cells
                .first()
                .map(|cell| derive_block_ram_program(cell, device, index))
                .unwrap_or_default(),
        ),
        SiteKind::Iob => SiteProgramKind::Iob(derive_iob_program(site, device, index)),
        SiteKind::Gclk => SiteProgramKind::Gclk,
        SiteKind::GclkIob => SiteProgramKind::GclkIob,
        SiteKind::Const | SiteKind::Unplaced | SiteKind::Unknown => return None,
    };
    Some(SiteProgram {
        tile_name: site.tile_name.clone(),
        tile_type: site.tile_type.clone(),
        site_kind: site.site_kind,
        site_name: site.site_name.clone(),
        x: site.x,
        y: site.y,
        kind,
    })
}

fn derive_slice_program(
    site: &SiteInstance,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> SliceProgram {
    let mut program = SliceProgram::default();

    for cell in &site.cells {
        let slot = slot_of_cell(cell, site);
        let primitive = cell.primitive_kind();
        if primitive.is_lut()
            && let Some(truth_table_bits) = preferred_lut_truth_table_bits(cell)
        {
            program.slots[slot].lut = Some(LutProgram {
                truth_table_bits,
                output_usage: slice_lut_output_usage(cell, device, index, slot),
            });
        }
        if primitive.is_sequential() {
            program.slots[slot].ff = Some(SequentialProgram {
                init: cell
                    .register_init_value()
                    .unwrap_or(SequentialInitValue::Low),
                data_path: slice_ff_data_path(cell, device, index, slot),
            });
            if slice_ff_uses_clock_enable(cell, device, index) {
                program.clock_enable_mode = SliceClockEnableMode::DirectCe;
            }
            if slice_ff_uses_set_reset(cell, device, index) {
                program.set_reset_mode = SliceSetResetMode::ActiveLowShared;
            }
        }
    }

    program
}

fn derive_iob_program(
    site: &SiteInstance,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> IobProgram {
    let Some(cell) = site.cells.first() else {
        return IobProgram::default();
    };

    IobProgram {
        input_used: device.nets.iter().any(|net| {
            net.driver.as_ref().is_some_and(|driver| {
                endpoint_matches_cell_pin(index, device, driver, &cell.cell_name, "IN")
            })
        }),
        output_used: device.nets.iter().any(|net| {
            net.sinks
                .iter()
                .any(|sink| endpoint_matches_cell_pin(index, device, sink, &cell.cell_name, "OUT"))
        }),
    }
}

fn slice_ff_uses_clock_enable(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> bool {
    device.nets.iter().any(|net| {
        net.sinks.iter().any(|sink| {
            matches!(
                index.resolve_endpoint_ref(device, sink),
                DeviceEndpointRef::Cell(sink_cell) if sink_cell.cell_name == cell.cell_name
            ) && cell.primitive_kind().is_clock_enable_pin(&sink.pin)
        })
    })
}

fn slice_ff_data_path(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
    slot: usize,
) -> SliceFfDataPath {
    let Some(driver) = device
        .nets
        .iter()
        .find(|net| {
            net.sinks
                .iter()
                .any(|sink| endpoint_matches_cell_pin(index, device, sink, &cell.cell_name, "D"))
        })
        .and_then(|net| net.driver.as_ref())
    else {
        return SliceFfDataPath::LocalLut;
    };

    match index.resolve_endpoint_ref(device, driver) {
        DeviceEndpointRef::Cell(driver_cell)
            if driver_cell.site_kind_class().is_logic_slice()
                && driver_cell.tile_name == cell.tile_name
                && driver_cell.site_name == cell.site_name
                && driver_cell.z == cell.z
                && driver_cell.primitive_kind().is_lut()
                && bel_slot(&driver_cell.bel).unwrap_or(usize::MAX).min(1) == slot =>
        {
            SliceFfDataPath::LocalLut
        }
        DeviceEndpointRef::Cell(_) | DeviceEndpointRef::Port(_) | DeviceEndpointRef::Unknown => {
            SliceFfDataPath::SiteBypass
        }
    }
}

fn slice_ff_uses_set_reset(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> bool {
    device.nets.iter().any(|net| {
        net.sinks.iter().any(|sink| {
            matches!(
                index.resolve_endpoint_ref(device, sink),
                DeviceEndpointRef::Cell(sink_cell) if sink_cell.cell_name == cell.cell_name
            ) && cell.primitive_kind().is_set_reset_pin(&sink.pin)
        })
    })
}

fn slice_lut_output_usage(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
    slot: usize,
) -> SliceLutOutputUsage {
    let has_routed_sink = device.nets.iter().any(|net| {
        net.driver.as_ref().is_some_and(|driver| {
            endpoint_matches_cell_pin(index, device, driver, &cell.cell_name, "O")
        }) && net
            .sinks
            .iter()
            .any(|sink| !is_paired_ff_d_sink(device, index, sink, cell, slot))
    });
    if has_routed_sink {
        SliceLutOutputUsage::RoutedOutput
    } else {
        SliceLutOutputUsage::HiddenLocalOnly
    }
}

fn is_paired_ff_d_sink(
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
    sink: &DeviceEndpoint,
    lut_cell: &DeviceCell,
    slot: usize,
) -> bool {
    sink.pin == "D"
        && matches!(
            index.resolve_endpoint_ref(device, sink),
            DeviceEndpointRef::Cell(ff_cell)
                if ff_cell.site_kind_class().is_logic_slice()
                    && ff_cell.primitive_kind().is_sequential()
                    && ff_cell.tile_name == lut_cell.tile_name
                    && ff_cell.site_name == lut_cell.site_name
                    && ff_cell.z == lut_cell.z
                    && bel_slot(&ff_cell.bel).unwrap_or(usize::MAX).min(1) == slot
        )
}

fn preferred_lut_truth_table_bits(cell: &DeviceCell) -> Option<Vec<u8>> {
    normalized_site_lut_truth_table_bits(
        cell_property(cell, "init"),
        cell_property(cell, "lut_init"),
        cell.primitive_kind(),
        LOGIC_SLICE_SITE_INPUTS,
    )
}

fn endpoint_matches_cell_pin(
    index: &DeviceDesignIndex<'_>,
    device: &DeviceDesign,
    endpoint: &DeviceEndpoint,
    cell_name: &str,
    pin_name: &str,
) -> bool {
    matches!(
        index.resolve_endpoint_ref(device, endpoint),
        DeviceEndpointRef::Cell(endpoint_cell) if endpoint_cell.cell_name == cell_name
    ) && endpoint.pin == pin_name
}

fn slot_of_cell(cell: &DeviceCell, site: &SiteInstance) -> usize {
    bel_slot(&cell.bel).unwrap_or(site.z.min(1)).min(1)
}

fn cell_property<'a>(cell: &'a DeviceCell, key: &str) -> Option<&'a str> {
    cell.property(key)
}

fn bel_slot(bel: &str) -> Option<usize> {
    bel.chars()
        .rev()
        .find(|ch| ch.is_ascii_digit())
        .and_then(|ch| ch.to_digit(10))
        .map(|digit| digit as usize)
}
