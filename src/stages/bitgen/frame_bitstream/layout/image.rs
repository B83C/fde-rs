use rustc_hash::FxHashMap as HashMap;

use crate::{
    cil::Cil,
    resource::{Arch, TileInstance},
    route::RouteBit,
};

use super::defaults::{apply_tile_site_defaults, apply_tile_transmission_defaults, env_flag};
use crate::stages::bitgen::frame_bitstream::model::{DEFAULT_FILL_BIT, TileFrameImage};

pub(super) fn build_arch_tile_images(
    arch: &Arch,
    cil: &Cil,
    transmission_defaults: &HashMap<String, Vec<RouteBit>>,
    notes: &mut Vec<String>,
) -> std::collections::BTreeMap<String, TileFrameImage> {
    let skip_site_defaults = env_flag("FDE_BITGEN_SKIP_SITE_DEFAULTS");
    let skip_transmission_defaults = env_flag("FDE_BITGEN_SKIP_TRANSMISSION_DEFAULTS");
    if skip_site_defaults {
        notes.push(
            "Skipping frame bitstream site defaults due to FDE_BITGEN_SKIP_SITE_DEFAULTS."
                .to_string(),
        );
    }
    if skip_transmission_defaults {
        notes.push(
            "Skipping frame bitstream transmission defaults due to FDE_BITGEN_SKIP_TRANSMISSION_DEFAULTS.".to_string(),
        );
    }
    let mut tiles_by_name = std::collections::BTreeMap::<String, TileFrameImage>::new();
    for tile in arch.tiles.values() {
        let Some(tile_def) = cil.tiles.get(&tile.tile_type) else {
            notes.push(format!(
                "Missing CIL tile definition for architecture tile {} ({}).",
                tile.name, tile.tile_type
            ));
            continue;
        };
        tiles_by_name.insert(tile.name.clone(), blank_frame_image(tile, tile_def));
    }
    for tile in arch.tiles.values() {
        let Some(tile_def) = cil.tiles.get(&tile.tile_type) else {
            continue;
        };
        if !skip_site_defaults {
            apply_tile_site_defaults(arch, &mut tiles_by_name, tile, tile_def, cil, notes);
        }
        if !skip_transmission_defaults {
            apply_tile_transmission_defaults(
                arch,
                &mut tiles_by_name,
                tile,
                tile_def,
                transmission_defaults,
                notes,
            );
        }
    }
    tiles_by_name
}

fn blank_frame_image(tile: &TileInstance, tile_def: &crate::cil::TileDef) -> TileFrameImage {
    TileFrameImage {
        tile_name: tile.name.clone(),
        tile_type: tile.tile_type.clone(),
        bit_x: tile.bit_x,
        bit_y: tile.bit_y,
        rows: tile_def.sram_rows,
        cols: tile_def.sram_cols,
        bits: vec![DEFAULT_FILL_BIT; tile_def.sram_rows.saturating_mul(tile_def.sram_cols)],
    }
}
