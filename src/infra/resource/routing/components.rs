use super::{ComponentBounds, RouteNode, StitchedComponentDb, TileStitchDb, WireInterner};
use crate::resource::Arch;
use std::collections::{HashMap, HashSet, VecDeque};

use super::stitch::stitched_neighbors;

pub(crate) fn build_stitched_components(
    db: &TileStitchDb,
    arch: &Arch,
    wires: &WireInterner,
) -> StitchedComponentDb {
    let mut bounds_by_node = HashMap::<RouteNode, ComponentBounds>::new();
    let mut representative_by_node = HashMap::<RouteNode, RouteNode>::new();
    let mut visited = HashSet::<RouteNode>::new();

    for tile in arch.tiles.values() {
        let Some(tile_stitch) = db.tiles.get(tile.tile_type.as_str()) else {
            continue;
        };
        for &wire in tile_stitch.net_ports.keys() {
            let start = RouteNode::new(tile.logic_x, tile.logic_y, wire);
            if !visited.insert(start) {
                continue;
            }

            let mut component = Vec::<RouteNode>::new();
            let mut bounds = ComponentBounds::new(&start);
            let mut queue = VecDeque::from([start]);
            while let Some(node) = queue.pop_front() {
                component.push(node);
                bounds.include(&node);
                for (next_x, next_y, next_wire) in stitched_neighbors(db, arch, wires, &node) {
                    let next = RouteNode::new(next_x, next_y, next_wire);
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }

            let representative = component
                .iter()
                .copied()
                .min()
                .expect("stitched component should not be empty");
            for node in component {
                bounds_by_node.insert(node, bounds);
                representative_by_node.insert(node, representative);
            }
        }
    }

    StitchedComponentDb {
        bounds_by_node,
        representative_by_node,
    }
}
