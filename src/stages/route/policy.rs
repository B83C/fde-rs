use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use smallvec::SmallVec;

use crate::domain::SiteKind;
use crate::{
    DeviceCell, domain::NetOrigin, resource::routing::WireMetadata, route::types::RouteNode,
};
#[cfg(test)]
use crate::{
    domain::{
        is_clock_distribution_wire_name, is_clock_sink_wire_name, is_directional_channel_wire_name,
        is_hex_like_wire_name, is_long_wire_name, is_pad_stub_wire_name,
    },
    route::types::{SiteRouteArc, WireInterner},
};

use super::{
    occupancy::{RouteNodeOwner, RouteSinkOwner, route_node_is_available, route_sink_is_available},
    router::{RouteNetKind, RouteSinkContext},
};

pub(super) struct NeighborAvailability<'a> {
    pub(super) stitched_components: &'a crate::resource::routing::StitchedComponentDb,
    pub(super) occupied_route_sinks: &'a HashMap<RouteNode, RouteSinkOwner>,
    pub(super) occupied_route_nodes: &'a HashMap<RouteNode, RouteNodeOwner>,
    pub(super) net_index: usize,
    pub(super) net_origin: NetOrigin,
    pub(super) tree_nodes: &'a HashSet<RouteNode>,
}

#[inline(always)]
pub(super) fn neighbor_is_available(
    availability: &NeighborAvailability<'_>,
    current: &RouteNode,
    neighbor: &RouteNode,
    local_arc: Option<usize>,
) -> bool {
    if availability.tree_nodes.contains(neighbor) {
        return true;
    }

    route_node_is_available(
        availability.stitched_components,
        availability.occupied_route_nodes,
        availability.net_index,
        neighbor,
        availability.tree_nodes,
    ) && route_sink_is_available(
        availability.occupied_route_sinks,
        availability.net_index,
        availability.net_origin,
        current,
        neighbor,
        local_arc,
    )
}

#[inline(always)]
pub(super) fn node_has_successors(context: &RouteSinkContext<'_>, node: &RouteNode) -> bool {
    if let Some(tile) = context.tile_context(node)
        && let Some(graph) = tile.graph
        && graph.has_adjacency(node.wire)
    {
        return true;
    }

    context.stitched_components.has_neighbors(node)
}

pub(super) fn neighbors(
    context: &RouteSinkContext<'_>,
    node: &RouteNode,
    net_kind: RouteNetKind,
    strict_clock_sink: bool,
) -> SmallVec<[(RouteNode, Option<usize>); 16]> {
    let mut result = SmallVec::new();
    let current_metadata = context.wires.metadata(node.wire);
    if let Some(tile) = context.tile_context(node)
        && let Some(graph) = tile.graph
    {
        for index in graph.adjacency(node.wire) {
            let Some(arc) = graph.arcs.get(*index) else {
                continue;
            };
            if !allow_clock_neighbor_metadata(
                net_kind,
                strict_clock_sink,
                current_metadata,
                context.wires.metadata(arc.to),
            ) {
                continue;
            }
            result.push((RouteNode::new(node.x, node.y, arc.to), Some(*index)));
        }
    }

    for &neighbor in context.stitched_components.neighbors(node) {
        let next_wire = neighbor.wire;
        if !allow_clock_neighbor_metadata(
            net_kind,
            strict_clock_sink,
            current_metadata,
            context.wires.metadata(next_wire),
        ) {
            continue;
        }
        result.push((neighbor, None));
    }

    result
}

pub(super) fn classify_route_net_kind(driver_cell: &DeviceCell) -> RouteNetKind {
    match driver_cell.site_kind_class() {
        SiteKind::Gclk => RouteNetKind::DedicatedClock,
        SiteKind::LogicSlice
        | SiteKind::Iob
        | SiteKind::GclkIob
        | SiteKind::Const
        | SiteKind::Unplaced
        | SiteKind::Unknown => RouteNetKind::Generic,
    }
}

#[cfg(test)]
pub(super) fn allow_clock_neighbor(
    net_kind: RouteNetKind,
    strict_clock_sink: bool,
    current_name: &str,
    next_name: &str,
) -> bool {
    if net_kind != RouteNetKind::DedicatedClock || !strict_clock_sink {
        return true;
    }

    if !is_clock_route_wire_name(current_name) {
        return false;
    }

    if is_clock_sink_wire_name(next_name) {
        return true;
    }

    is_clock_route_wire_name(next_name)
}

fn allow_clock_neighbor_metadata(
    net_kind: RouteNetKind,
    strict_clock_sink: bool,
    current: WireMetadata,
    next: WireMetadata,
) -> bool {
    if net_kind != RouteNetKind::DedicatedClock || !strict_clock_sink {
        return true;
    }

    if !is_clock_route_wire_metadata(current) {
        return false;
    }

    if next.is_clock_sink() {
        return true;
    }

    is_clock_route_wire_metadata(next)
}

#[cfg(test)]
fn is_clock_route_wire_name(raw: &str) -> bool {
    // C++ routed dedicated clocks do not stay on GCLK-only wires. Real baseline
    // paths fan out through LLH/H6/V6/channel/pad-stub branches before entering
    // *_CLK_B sinks, so the legality filter must accept those branch families.
    is_clock_distribution_wire_name(raw)
        || is_long_wire_name(raw)
        || is_hex_like_wire_name(raw)
        || is_directional_channel_wire_name(raw)
        || is_pad_stub_wire_name(raw)
}

fn is_clock_route_wire_metadata(metadata: WireMetadata) -> bool {
    metadata.is_clock_distribution()
        || metadata.is_long()
        || metadata.is_hex_like()
        || metadata.is_directional_channel()
        || metadata.is_pad_stub()
}

#[cfg(test)]
pub(super) fn should_skip_local_arc(
    tile: &crate::route::lookup::TileRouteContext<'_>,
    arc: &SiteRouteArc,
    wires: &WireInterner,
) -> bool {
    if tile.site_type != "GSB_LFT" {
        return false;
    }

    let from = wires.resolve(arc.from);
    let to = wires.resolve(arc.to);
    to.starts_with("LEFT_O") && from.starts_with("LEFT_H6") && from.contains("_BUF")
}

#[cfg(test)]
mod tests {
    use crate::route::{
        lookup::TileRouteContext,
        router::RouteNetKind,
        types::{SiteRouteArc, WireInterner},
    };

    use super::{allow_clock_neighbor, should_skip_local_arc};

    #[test]
    fn blocks_left_h6_buffer_arcs_into_left_o1() {
        let mut wires = WireInterner::default();
        let tile = TileRouteContext {
            tile_name: "LR5",
            tile_type: "LR5",
            site_name: "GSB_LFT",
            site_type: "GSB_LFT",
        };
        let blocked = SiteRouteArc {
            from: wires.intern("LEFT_H6A_BUF1"),
            to: wires.intern("LEFT_O1"),
            basic_cell: "SPS_O1".to_string(),
            bits: Vec::new(),
        };
        let allowed = SiteRouteArc {
            from: wires.intern("LEFT_E_BUF3"),
            to: wires.intern("LEFT_O1"),
            basic_cell: "SPS_O1".to_string(),
            bits: Vec::new(),
        };

        assert!(should_skip_local_arc(&tile, &blocked, &wires));
        assert!(!should_skip_local_arc(&tile, &allowed, &wires));
    }

    #[test]
    fn blocks_left_h6_buffer_arcs_into_all_left_outputs() {
        let mut wires = WireInterner::default();
        let tile = TileRouteContext {
            tile_name: "LR5",
            tile_type: "LR5",
            site_name: "GSB_LFT",
            site_type: "GSB_LFT",
        };
        let blocked_o2 = SiteRouteArc {
            from: wires.intern("LEFT_H6E_BUF2"),
            to: wires.intern("LEFT_O2"),
            basic_cell: "SPS_O2".to_string(),
            bits: Vec::new(),
        };
        let blocked_o3 = SiteRouteArc {
            from: wires.intern("LEFT_H6A_BUF3"),
            to: wires.intern("LEFT_O3"),
            basic_cell: "SPS_O3".to_string(),
            bits: Vec::new(),
        };

        assert!(should_skip_local_arc(&tile, &blocked_o2, &wires));
        assert!(should_skip_local_arc(&tile, &blocked_o3, &wires));
    }

    #[test]
    fn strict_clock_sink_keeps_cpp_clock_branch_wires_available() {
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "CLKB_GCLK1_PW",
            "CLKB_GCLK1",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "CLKC_HGCLK1",
            "CLKC_VGCLK1",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "CLKV_VGCLK1",
            "CLKV_GCLK_BUFR1",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "GCLK1",
            "S0_CLK_B",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "CLKB_GCLK1_PW",
            "CLKB_LLH1",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "GCLK1",
            "W_P16",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "V6N2",
            "S0_CLK_B",
        ));
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "E_P15",
            "S0_CLK_B",
        ));
        assert!(!allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            true,
            "OUT4",
            "S0_CLK_B",
        ));
    }

    #[test]
    fn mixed_use_gclk_nets_keep_generic_escape_available() {
        assert!(allow_clock_neighbor(
            RouteNetKind::DedicatedClock,
            false,
            "CLKB_GCLK1_PW",
            "CLKB_LLH1",
        ));
    }
}
