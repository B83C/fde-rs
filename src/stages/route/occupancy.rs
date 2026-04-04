use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::domain::NetOrigin;
use crate::resource::routing::StitchedComponentDb;

use super::types::{RouteNode, RoutedPip, WireId};
type RouteWireKey = RouteNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RouteSinkOwner {
    pub(super) net_index: usize,
    pub(super) origin: NetOrigin,
    pub(super) from: WireId,
}

pub(super) type RouteNodeOwner = usize;

#[inline(always)]
pub(super) fn route_sink_is_available(
    occupied_route_sinks: &HashMap<RouteWireKey, RouteSinkOwner>,
    net_index: usize,
    net_origin: NetOrigin,
    current: &RouteNode,
    neighbor: &RouteNode,
    local_arc: Option<usize>,
) -> bool {
    let Some(_) = local_arc else {
        return true;
    };
    occupied_route_sinks
        .get(neighbor)
        .map(|owner| {
            owner.net_index == net_index
                || (owner.from == current.wire
                    && (owner.origin == NetOrigin::SyntheticGclk
                        || net_origin == NetOrigin::SyntheticGclk))
        })
        .unwrap_or(true)
}

#[inline(always)]
pub(super) fn route_node_is_available(
    stitched_components: &StitchedComponentDb,
    occupied_route_nodes: &HashMap<RouteNode, RouteNodeOwner>,
    net_index: usize,
    neighbor: &RouteNode,
    tree_nodes: &HashSet<RouteNode>,
) -> bool {
    if tree_nodes.contains(neighbor) {
        return true;
    }

    occupied_route_nodes
        .get(&stitched_components.occupancy_key(neighbor))
        .map(|owner| *owner == net_index)
        .unwrap_or(true)
}

pub(super) fn reserve_route_sinks(
    occupied_route_sinks: &mut HashMap<RouteWireKey, RouteSinkOwner>,
    net_index: usize,
    net_origin: NetOrigin,
    path: &[RoutedPip],
) {
    for pip in path {
        occupied_route_sinks
            .entry(RouteNode::new(pip.x, pip.y, pip.to))
            .or_insert(RouteSinkOwner {
                net_index,
                origin: net_origin,
                from: pip.from,
            });
    }
}

pub(super) fn reserve_route_nodes(
    stitched_components: &StitchedComponentDb,
    occupied_route_nodes: &mut HashMap<RouteNode, RouteNodeOwner>,
    net_index: usize,
    path_nodes: &[RouteNode],
) {
    for &node in path_nodes {
        occupied_route_nodes
            .entry(stitched_components.occupancy_key(&node))
            .or_insert(net_index);
    }
}
