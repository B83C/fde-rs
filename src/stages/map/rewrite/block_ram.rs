use crate::{
    domain::{
        BlockRamKind, BlockRamPin, block_ram_port_attr, parse_ramb4_dual_port_widths,
        parse_ramb4_single_port_width,
    },
    ir::Cell,
};

#[derive(Debug, Clone)]
pub(super) struct BlockRamRewrite {
    pub(super) canonical_type: &'static str,
    pub(super) pin_renames: Vec<(String, String)>,
    pub(super) property_writes: Vec<(&'static str, String)>,
}

pub(super) fn block_ram_rewrite(cell: &Cell) -> Option<BlockRamRewrite> {
    if !cell.is_block_ram() {
        return None;
    }

    let type_name = cell.type_name.trim();
    if BlockRamKind::from_type_name(type_name).is_some() {
        return None;
    }

    if type_name.eq_ignore_ascii_case("BLOCKRAM_SINGLE_PORT") {
        return Some(BlockRamRewrite {
            canonical_type: BlockRamKind::SinglePort.canonical_type_name(),
            pin_renames: canonical_pin_renames(cell, BlockRamKind::SinglePort, 0, 0),
            property_writes: Vec::new(),
        });
    }

    if type_name.eq_ignore_ascii_case("BLOCKRAM_DUAL_PORT") {
        return Some(BlockRamRewrite {
            canonical_type: BlockRamKind::DualPort.canonical_type_name(),
            pin_renames: canonical_pin_renames(cell, BlockRamKind::DualPort, 0, 0),
            property_writes: Vec::new(),
        });
    }

    if let Some(width) = parse_ramb4_single_port_width(type_name) {
        let addr_shift = width.trailing_zeros() as usize;
        return Some(BlockRamRewrite {
            canonical_type: BlockRamKind::SinglePort.canonical_type_name(),
            pin_renames: canonical_pin_renames(cell, BlockRamKind::SinglePort, addr_shift, 0),
            property_writes: vec![("PORT_ATTR", block_ram_port_attr(width))],
        });
    }

    let (width_a, width_b) = parse_ramb4_dual_port_widths(type_name)?;
    Some(BlockRamRewrite {
        canonical_type: BlockRamKind::DualPort.canonical_type_name(),
        pin_renames: canonical_pin_renames(
            cell,
            BlockRamKind::DualPort,
            width_a.trailing_zeros() as usize,
            width_b.trailing_zeros() as usize,
        ),
        property_writes: vec![
            ("PORTA_ATTR", block_ram_port_attr(width_a)),
            ("PORTB_ATTR", block_ram_port_attr(width_b)),
        ],
    })
}

fn canonical_pin_renames(
    cell: &Cell,
    kind: BlockRamKind,
    addr_shift_a: usize,
    addr_shift_b: usize,
) -> Vec<(String, String)> {
    cell.inputs
        .iter()
        .chain(cell.outputs.iter())
        .filter_map(|pin| {
            let canonical = BlockRamPin::parse(&pin.port)?.canonical_map_name(
                kind,
                addr_shift_a,
                addr_shift_b,
            )?;
            (!pin.port.eq_ignore_ascii_case(&canonical)).then(|| (pin.port.clone(), canonical))
        })
        .collect()
}
