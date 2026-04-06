mod bram;
mod clock;
mod io;
mod shared;
mod slice;
#[cfg(test)]
mod tests;

use crate::{
    bitgen::{DeviceCell, DeviceEndpoint, DeviceNet},
    domain::SiteKind,
};

use self::{
    bram::{bram_sink_nets, bram_source_nets},
    clock::{gclk_sink_nets, gclk_source_nets, gclkiob_source_nets},
    io::{iob_sink_nets, iob_source_nets},
    slice::{slice_sink_nets, slice_source_nets},
};
use super::types::WireInterner;

pub(crate) use shared::WireSet;
pub(crate) use slice::{should_skip_unmapped_sink, sink_requires_all_wires};

pub(crate) fn should_route_device_net(net: &DeviceNet) -> bool {
    if net.origin_kind().is_synthetic_pad() {
        return false;
    }
    net.driver.as_ref().is_some_and(DeviceEndpoint::is_cell)
        && net.sinks.iter().any(DeviceEndpoint::is_cell)
}

pub(crate) fn endpoint_source_nets(
    cell: &DeviceCell,
    endpoint: &DeviceEndpoint,
    wires: &mut WireInterner,
) -> WireSet {
    match cell.site_kind_class() {
        SiteKind::LogicSlice => slice_source_nets(cell, endpoint, wires),
        SiteKind::BlockRam => bram_source_nets(cell, endpoint, wires),
        SiteKind::Iob => iob_source_nets(cell, endpoint, wires),
        SiteKind::GclkIob => gclkiob_source_nets(cell, endpoint, wires),
        SiteKind::Gclk => gclk_source_nets(cell, endpoint, wires),
        SiteKind::Const | SiteKind::Unplaced | SiteKind::Unknown => WireSet::new(),
    }
}

pub(crate) fn endpoint_sink_nets(
    driver_cell: Option<&DeviceCell>,
    cell: &DeviceCell,
    endpoint: &DeviceEndpoint,
    wires: &mut WireInterner,
) -> WireSet {
    match cell.site_kind_class() {
        SiteKind::LogicSlice => slice_sink_nets(driver_cell, cell, endpoint, wires),
        SiteKind::BlockRam => bram_sink_nets(cell, endpoint, wires),
        SiteKind::Iob => iob_sink_nets(cell, endpoint, wires),
        SiteKind::Gclk => gclk_sink_nets(cell, endpoint, wires),
        SiteKind::GclkIob | SiteKind::Const | SiteKind::Unplaced | SiteKind::Unknown => {
            WireSet::new()
        }
    }
}
