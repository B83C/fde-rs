use super::shared::WireSet;
use crate::{DeviceCell, DeviceEndpoint};

use super::super::types::WireInterner;

pub(super) fn bram_source_nets(
    _cell: &DeviceCell,
    endpoint: &DeviceEndpoint,
    wires: &mut WireInterner,
) -> WireSet {
    let mut set = WireSet::new();
    set.push(wires.intern(&normalized_bram_wire_name(&endpoint.pin)));
    set
}

pub(super) fn bram_sink_nets(
    _cell: &DeviceCell,
    endpoint: &DeviceEndpoint,
    wires: &mut WireInterner,
) -> WireSet {
    let mut set = WireSet::new();
    set.push(wires.intern(&normalized_bram_wire_name(&endpoint.pin)));
    set
}

fn normalized_bram_wire_name(pin: &str) -> String {
    pin.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}
