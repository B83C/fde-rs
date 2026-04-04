use super::{ComponentBounds, RouteNode, StitchedComponentDb, TileStitchDb, WireInterner};
use crate::resource::Arch;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::VecDeque;

use super::stitch::stitched_neighbors;

pub(crate) fn build_stitched_components(
    db: &TileStitchDb,
    arch: &Arch,
    wires: &WireInterner,
) -> StitchedComponentDb {
    let mut bounds_by_node = HashMap::<RouteNode, ComponentBounds>::default();
    let mut neighbors_by_node = HashMap::default();
    let mut representative_by_node = HashMap::<RouteNode, RouteNode>::default();
    let mut visited = HashSet::<RouteNode>::default();

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
                let neighbors = stitched_neighbors(db, arch, wires, &node)
                    .into_iter()
                    .map(|(next_x, next_y, next_wire)| RouteNode::new(next_x, next_y, next_wire))
                    .collect::<smallvec::SmallVec<[RouteNode; 8]>>();
                for next in &neighbors {
                    if visited.insert(*next) {
                        queue.push_back(*next);
                    }
                }
                neighbors_by_node.insert(node, neighbors);
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
        neighbors_by_node,
        representative_by_node,
    }
}
