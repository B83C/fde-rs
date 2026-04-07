use super::super::types::BlockRamProgram;
use crate::{
    bitgen::{DeviceCell, DeviceDesign, DeviceDesignIndex, DeviceEndpoint, DeviceEndpointRef},
    domain::{
        BlockRamControlSignal, BlockRamKind, BlockRamPin, BlockRamPortSide,
        normalized_block_ram_init_property_key,
    },
};

pub(super) fn derive_block_ram_program(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
) -> BlockRamProgram {
    let Some(kind) = BlockRamKind::from_type_name(&cell.type_name) else {
        return BlockRamProgram::default();
    };

    BlockRamProgram {
        port_a_attr: port_attr_value(cell, kind, BlockRamPortSide::A),
        port_b_attr: port_attr_value(cell, kind, BlockRamPortSide::B),
        clka_used: block_ram_sink_pin_used(
            cell,
            device,
            index,
            BlockRamPin::Control {
                side: BlockRamPortSide::A,
                signal: BlockRamControlSignal::Clock,
            },
        ),
        clkb_used: kind == BlockRamKind::DualPort
            && block_ram_sink_pin_used(
                cell,
                device,
                index,
                BlockRamPin::Control {
                    side: BlockRamPortSide::B,
                    signal: BlockRamControlSignal::Clock,
                },
            ),
        ena_used: block_ram_sink_pin_used(
            cell,
            device,
            index,
            BlockRamPin::Control {
                side: BlockRamPortSide::A,
                signal: BlockRamControlSignal::Enable,
            },
        ),
        enb_used: kind == BlockRamKind::DualPort
            && block_ram_sink_pin_used(
                cell,
                device,
                index,
                BlockRamPin::Control {
                    side: BlockRamPortSide::B,
                    signal: BlockRamControlSignal::Enable,
                },
            ),
        wea_used: block_ram_sink_pin_used(
            cell,
            device,
            index,
            BlockRamPin::Control {
                side: BlockRamPortSide::A,
                signal: BlockRamControlSignal::WriteEnable,
            },
        ),
        web_used: kind == BlockRamKind::DualPort
            && block_ram_sink_pin_used(
                cell,
                device,
                index,
                BlockRamPin::Control {
                    side: BlockRamPortSide::B,
                    signal: BlockRamControlSignal::WriteEnable,
                },
            ),
        rsta_used: block_ram_sink_pin_used(
            cell,
            device,
            index,
            BlockRamPin::Control {
                side: BlockRamPortSide::A,
                signal: BlockRamControlSignal::Reset,
            },
        ),
        rstb_used: kind == BlockRamKind::DualPort
            && block_ram_sink_pin_used(
                cell,
                device,
                index,
                BlockRamPin::Control {
                    side: BlockRamPortSide::B,
                    signal: BlockRamControlSignal::Reset,
                },
            ),
        init_words: block_ram_init_words(cell),
    }
}

fn port_attr_value(
    cell: &DeviceCell,
    kind: BlockRamKind,
    side: BlockRamPortSide,
) -> Option<String> {
    match (kind, side) {
        (BlockRamKind::SinglePort, BlockRamPortSide::A) => cell_property(cell, "PORTA_ATTR")
            .or_else(|| cell_property(cell, "PORT_ATTR"))
            .map(str::to_owned),
        (BlockRamKind::SinglePort, BlockRamPortSide::B) => None,
        (BlockRamKind::DualPort, BlockRamPortSide::A) => {
            cell_property(cell, "PORTA_ATTR").map(str::to_owned)
        }
        (BlockRamKind::DualPort, BlockRamPortSide::B) => {
            cell_property(cell, "PORTB_ATTR").map(str::to_owned)
        }
    }
}

fn block_ram_sink_pin_used(
    cell: &DeviceCell,
    device: &DeviceDesign,
    index: &DeviceDesignIndex<'_>,
    expected_pin: BlockRamPin,
) -> bool {
    device.nets.iter().any(|net| {
        net.sinks.iter().any(|sink| {
            endpoint_matches_cell(index, device, sink, &cell.cell_name)
                && BlockRamPin::parse(&sink.pin) == Some(expected_pin)
        })
    })
}

fn endpoint_matches_cell(
    index: &DeviceDesignIndex<'_>,
    device: &DeviceDesign,
    endpoint: &DeviceEndpoint,
    cell_name: &str,
) -> bool {
    matches!(
        index.resolve_endpoint_ref(device, endpoint),
        DeviceEndpointRef::Cell(endpoint_cell) if endpoint_cell.cell_name == cell_name
    )
}

fn cell_property<'a>(cell: &'a DeviceCell, key: &str) -> Option<&'a str> {
    cell.properties
        .iter()
        .find(|property| property.key.eq_ignore_ascii_case(key))
        .map(|property| property.value.as_str())
}

fn block_ram_init_words(cell: &DeviceCell) -> Vec<(String, String)> {
    let mut words = cell
        .properties
        .iter()
        .filter_map(|property| {
            normalized_block_ram_init_property_key(&property.key)
                .map(|key| (key, property.value.clone()))
        })
        .collect::<Vec<_>>();
    words.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
    words
}
