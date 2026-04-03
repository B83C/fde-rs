use crate::{bitgen::TileBitAssignment, cil::Cil, resource::Arch, route::DeviceRoutePip};

use super::super::lookup::find_route_sram;
use super::{
    image::ConfigImageBuilder,
    target::{SourceTileContext, resolve_target_assignment},
};

pub(super) fn encode_route_pip(
    image: &mut ConfigImageBuilder,
    pip: &DeviceRoutePip,
    cil: &Cil,
    arch: Option<&Arch>,
) {
    let Some(tile_def) = cil.tiles.get(&pip.tile_type) else {
        image.note(format!(
            "Missing CIL tile definition for routed pip {}:{} -> {}:{}.",
            pip.tile_type, pip.site_name, pip.from_net, pip.to_net
        ));
        return;
    };
    let Some(tile_site) = cil.tile_transmission_site(&pip.tile_type, &pip.site_name) else {
        image.note(format!(
            "Missing transmission-site mapping for {}:{}.",
            pip.tile_type, pip.site_name
        ));
        return;
    };

    let source = SourceTileContext {
        tile_name: &pip.tile_name,
        tile_type: &pip.tile_type,
        x: pip.x,
        y: pip.y,
        rows: tile_def.sram_rows,
        cols: tile_def.sram_cols,
    };

    image.register_config(source, &pip.site_name, &pip.from_net, &pip.to_net);

    for bit in &pip.bits {
        let Some(mapping) = find_route_sram(tile_site, bit) else {
            image.note(format!(
                "Missing route SRAM mapping for {}:{}:{} on {}:{}.",
                pip.from_net, bit.basic_cell, bit.sram_name, pip.tile_type, pip.site_name
            ));
            continue;
        };
        let mut notes = Vec::new();
        let Some(target) = resolve_target_assignment(
            cil,
            arch,
            source,
            mapping,
            &format!(
                "{}:{}:{} on {}:{}",
                pip.from_net, bit.basic_cell, bit.sram_name, pip.tile_type, pip.site_name
            ),
            &mut notes,
        ) else {
            image.extend_notes(notes);
            continue;
        };
        image.extend_notes(notes);
        image.insert_assignment(
            &target,
            TileBitAssignment {
                site_name: pip.site_name.clone(),
                cfg_name: pip.from_net.clone(),
                function_name: pip.to_net.clone(),
                basic_cell: bit.basic_cell.clone(),
                sram_name: bit.sram_name.clone(),
                row: target.row,
                col: target.col,
                value: bit.value,
            },
        );
    }
}
