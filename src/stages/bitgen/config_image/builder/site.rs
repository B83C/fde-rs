use crate::{bitgen::ProgrammedSite, bitgen::TileBitAssignment, cil::Cil, resource::Arch};

use super::super::{lookup::find_tile_sram, resolve::resolve_site_config, types::ConfigResolution};
use super::{
    image::ConfigImageBuilder,
    target::{SourceTileContext, resolve_target_assignment},
};

pub(super) fn encode_programmed_site(
    image: &mut ConfigImageBuilder,
    site: &ProgrammedSite,
    cil: &Cil,
    arch: Option<&Arch>,
) {
    let Some(tile_def) = cil.tiles.get(&site.tile_type) else {
        image.note(format!(
            "Missing CIL tile definition for {} on site {}.",
            site.tile_type, site.site_name
        ));
        return;
    };
    let Some(site_def) = cil.site_def(site.site_kind) else {
        image.note(format!(
            "Missing CIL site definition for {} on tile {}.",
            site.site_kind.as_str(),
            site.tile_name
        ));
        return;
    };
    let Some(tile_site) = cil.tile_site(&site.tile_type, &site.site_name) else {
        image.note(format!(
            "Missing tile-site mapping for {}:{}.",
            site.tile_type, site.site_name
        ));
        return;
    };

    let source = SourceTileContext {
        tile_name: &site.tile_name,
        tile_type: &site.tile_type,
        x: site.x,
        y: site.y,
        rows: tile_def.sram_rows,
        cols: tile_def.sram_cols,
    };

    for request in &site.requests {
        match resolve_site_config(site_def, &request.cfg_name, &request.function_name) {
            ConfigResolution::Matched(bits) => {
                image.register_config(
                    source,
                    &site.site_name,
                    &request.cfg_name,
                    &request.function_name,
                );
                for bit in bits {
                    let Some(mapping) = find_tile_sram(tile_site, &bit) else {
                        image.note(format!(
                            "Missing site SRAM mapping for {}:{}:{} on {}:{}.",
                            bit.cfg_name,
                            bit.basic_cell,
                            bit.sram_name,
                            site.tile_type,
                            site.site_name
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
                            bit.cfg_name,
                            bit.basic_cell,
                            bit.sram_name,
                            site.tile_type,
                            site.site_name
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
                            site_name: site.site_name.clone(),
                            cfg_name: bit.cfg_name.clone(),
                            function_name: bit.function_name.clone(),
                            basic_cell: bit.basic_cell.clone(),
                            sram_name: bit.sram_name.clone(),
                            row: target.row,
                            col: target.col,
                            value: bit.value,
                        },
                    );
                }
            }
            ConfigResolution::Unmatched if request.function_name == "#OFF" => {}
            ConfigResolution::Unmatched => image.note(format!(
                "Unresolved config {}={} for {}:{}.",
                request.cfg_name, request.function_name, site.tile_type, site.site_name
            )),
        }
    }
}
