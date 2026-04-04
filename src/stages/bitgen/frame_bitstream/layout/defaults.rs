use rustc_hash::FxHashMap as HashMap;

use crate::{
    cil::Cil,
    domain::SiteKind,
    resource::{Arch, TileInstance},
    route::RouteBit,
};

use crate::stages::bitgen::config_image::{
    ConfigResolution, find_route_sram, find_tile_sram, resolve_site_config,
};
use crate::stages::bitgen::frame_bitstream::model::TileFrameImage;

use super::apply::apply_mapped_bit;

pub(super) fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value != "0")
}

pub(super) fn apply_tile_site_defaults(
    arch: &Arch,
    tiles_by_name: &mut std::collections::BTreeMap<String, TileFrameImage>,
    source_tile: &TileInstance,
    tile_def: &crate::cil::TileDef,
    cil: &Cil,
    notes: &mut Vec<String>,
) {
    for tile_site in tile_def
        .clusters
        .iter()
        .flat_map(|cluster| cluster.sites.iter())
    {
        if tile_site.srams.is_empty() {
            continue;
        }
        let site_kind = SiteKind::classify(&tile_site.site_type);
        let Some(site_def) = cil
            .sites
            .get(&tile_site.site_type)
            .or_else(|| cil.site_def(site_kind))
        else {
            continue;
        };
        for cfg in &site_def.config_elements {
            let Some(default_function) = cfg.default_function() else {
                continue;
            };
            if default_function.srams.is_empty() {
                continue;
            }
            let ConfigResolution::Matched(bits) =
                resolve_site_config(site_def, &cfg.name, &default_function.name)
            else {
                notes.push(format!(
                    "Could not resolve default config {}={} for {}:{}.",
                    cfg.name, default_function.name, tile_def.name, tile_site.name
                ));
                continue;
            };
            for bit in bits {
                let Some(mapping) = find_tile_sram(tile_site, &bit) else {
                    notes.push(format!(
                        "Missing default site SRAM mapping for {}:{}:{} on {}:{}.",
                        bit.cfg_name, bit.basic_cell, bit.sram_name, tile_def.name, tile_site.name
                    ));
                    continue;
                };
                let context = format!(
                    "default site bit {}:{}:{} on {}:{}",
                    bit.cfg_name, bit.basic_cell, bit.sram_name, tile_def.name, tile_site.name
                );
                apply_mapped_bit(
                    arch,
                    tiles_by_name,
                    source_tile,
                    mapping,
                    bit.value,
                    &context,
                    notes,
                );
            }
        }
    }
}

pub(super) fn apply_tile_transmission_defaults(
    arch: &Arch,
    tiles_by_name: &mut std::collections::BTreeMap<String, TileFrameImage>,
    source_tile: &TileInstance,
    tile_def: &crate::cil::TileDef,
    transmission_defaults: &HashMap<String, Vec<RouteBit>>,
    notes: &mut Vec<String>,
) {
    for transmission_site in tile_def
        .transmissions
        .iter()
        .flat_map(|transmission| transmission.sites.iter())
    {
        if transmission_site.srams.is_empty() {
            continue;
        }
        let Some(default_bits) = transmission_defaults.get(&transmission_site.site_type) else {
            continue;
        };
        for bit in default_bits {
            let Some(mapping) = find_route_sram(transmission_site, bit) else {
                notes.push(format!(
                    "Missing default transmission SRAM mapping for {}:{} on {}:{}.",
                    bit.basic_cell, bit.sram_name, tile_def.name, transmission_site.name
                ));
                continue;
            };
            let context = format!(
                "default transmission bit {}:{} on {}:{}",
                bit.basic_cell, bit.sram_name, tile_def.name, transmission_site.name
            );
            apply_mapped_bit(
                arch,
                tiles_by_name,
                source_tile,
                mapping,
                bit.value,
                &context,
                notes,
            );
        }
    }
}
