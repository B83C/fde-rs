mod apply;
mod defaults;
mod image;
#[cfg(test)]
mod tests;

use rustc_hash::FxHashMap as HashMap;

use crate::{bitgen::ConfigImage, cil::Cil, route::RouteBit};

use self::{apply::apply_config_assignments, image::build_arch_tile_images};
use super::model::TileColumns;

pub(crate) fn build_tile_columns(
    arch: &crate::resource::Arch,
    cil: &Cil,
    config_image: &ConfigImage,
    transmission_defaults: &HashMap<String, Vec<RouteBit>>,
    notes: &mut Vec<String>,
) -> TileColumns {
    let mut tiles_by_name = build_arch_tile_images(arch, cil, transmission_defaults, notes);
    apply_config_assignments(arch, cil, &mut tiles_by_name, config_image, notes);

    let mut columns = TileColumns::new();
    for image in tiles_by_name.into_values() {
        columns.entry(image.bit_y).or_default().push(image);
    }
    for tiles in columns.values_mut() {
        tiles.sort_by(|lhs, rhs| {
            (lhs.bit_x, lhs.tile_name.as_str()).cmp(&(rhs.bit_x, rhs.tile_name.as_str()))
        });
    }
    columns
}
