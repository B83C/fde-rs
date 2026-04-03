use anyhow::Result;
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet};

use super::cost::{route_heuristic, route_transition_cost};
use super::endpoint::{ResolvedRouteEndpoint, resolve_route_endpoint};
use super::guide::{GuideDistances, GuideRouteMode, GuidedRouteNode, OrderedGuide, guide_penalty};
use super::heap::{frontier_heap_pop, frontier_heap_push};
use super::occupancy::{RouteNodeOwner, RouteSinkOwner, reserve_route_nodes, reserve_route_sinks};
use super::policy::{
    NeighborAvailability, classify_route_net_kind, neighbor_is_available, neighbors,
};

use super::{
    lookup::route_context_for_node,
    mapping::{
        endpoint_sink_nets, endpoint_source_nets, should_route_device_net,
        should_skip_unmapped_sink, sink_requires_all_wires,
    },
    types::{
        DeviceRouteImage, DeviceRoutePip, RouteNode, RoutedPip, SearchParentStep, SearchState,
        SiteRouteGraphs, WireId, WireInterner,
    },
    wire::tile_distance,
};
use crate::{
    DeviceDesign, DeviceDesignIndex,
    cil::Cil,
    domain::{NetOrigin, is_clock_sink_wire_name},
    resource::{
        Arch,
        routing::{
            StitchedComponentDb, TileStitchDb, build_stitched_components, load_site_route_graphs,
            load_tile_stitch_db,
        },
    },
};

pub fn route_device_design(
    device: &DeviceDesign,
    arch: &Arch,
    arch_path: &std::path::Path,
    cil: &Cil,
) -> Result<DeviceRouteImage> {
    let mut wires = WireInterner::default();
    let graphs = load_site_route_graphs(arch_path, cil, &mut wires)?;
    let stitch_db = load_tile_stitch_db(arch_path, &mut wires)?;
    let stitched_components = build_stitched_components(&stitch_db, arch, &wires);
    let index = DeviceDesignIndex::build(device);

    let mut pips = Vec::new();
    let mut notes = Vec::new();
    let mut guide_usage = GuideUsageStats::default();
    let mut occupied_route_sinks = HashMap::<(usize, usize, WireId), RouteSinkOwner>::new();
    let mut occupied_route_nodes = HashMap::<RouteNode, RouteNodeOwner>::new();
    let context = RouteSinkContext {
        arch,
        cil,
        graphs: &graphs,
        stitch_db: &stitch_db,
        stitched_components: &stitched_components,
        wires: &mut wires,
    };

    let mut net_order = (0..device.nets.len()).collect::<Vec<_>>();
    net_order.sort_by_key(|&net_index| route_net_order_key(device, &index, net_index));

    for net_index in net_order {
        let net = &device.nets[net_index];
        if !should_route_device_net(net) {
            continue;
        }

        let Some(driver) = net.driver.as_ref() else {
            notes.push(format!("Net {} has no routed driver.", net.name));
            continue;
        };

        let driver_cell = match resolve_route_endpoint(device, &index, driver) {
            ResolvedRouteEndpoint::Cell(cell) => cell,
            ResolvedRouteEndpoint::Port(port) => {
                notes.push(format!(
                    "Net {} driver {} resolves to device port {} and is not a routable cell.",
                    net.name, driver.name, port.port_name
                ));
                continue;
            }
            ResolvedRouteEndpoint::Unknown => {
                notes.push(format!(
                    "Net {} driver {} is not a routable cell.",
                    net.name, driver.name
                ));
                continue;
            }
        };

        let net_kind = classify_route_net_kind(driver_cell);
        let net_origin = net.origin_kind();
        let source_nets = endpoint_source_nets(driver_cell, driver, context.wires);
        if source_nets.is_empty() {
            notes.push(format!(
                "Net {} driver {}:{} has no route-source mapping.",
                net.name, driver.name, driver.pin
            ));
            continue;
        }

        let roots = source_nets
            .iter()
            .copied()
            .map(|wire| RouteNode::new(driver.x, driver.y, wire))
            .collect::<Vec<_>>();
        let mut tree_nodes = roots.iter().copied().collect::<HashSet<_>>();
        let mut used_pips = HashSet::<(usize, usize, WireId, WireId)>::new();

        let mut sinks = net.sinks.iter().collect::<Vec<_>>();
        // Prefer same-site cell sinks before remote or port sinks. This keeps
        // local feedback branches available for later remote sinks instead of
        // forcing the whole net through the first long-distance escape path.
        sinks.sort_by_key(|sink| {
            let sink_class = match resolve_route_endpoint(device, &index, sink) {
                ResolvedRouteEndpoint::Cell(cell)
                    if cell.tile_name == driver_cell.tile_name
                        && cell.site_name == driver_cell.site_name =>
                {
                    0u8
                }
                ResolvedRouteEndpoint::Cell(_) => 1u8,
                ResolvedRouteEndpoint::Port(_) => 2u8,
                ResolvedRouteEndpoint::Unknown => 3u8,
            };
            (
                sink_class,
                std::cmp::Reverse(net.guide_tiles_for_sink(sink).len()),
                tile_distance(driver.x, driver.y, sink.x, sink.y),
            )
        });

        for sink in sinks {
            let sink_cell = match resolve_route_endpoint(device, &index, sink) {
                ResolvedRouteEndpoint::Cell(cell) => cell,
                ResolvedRouteEndpoint::Port(port) => {
                    notes.push(format!(
                        "Net {} sink {} resolves to device port {} and is not a routable cell.",
                        net.name, sink.name, port.port_name
                    ));
                    continue;
                }
                ResolvedRouteEndpoint::Unknown => {
                    notes.push(format!(
                        "Net {} sink {} is not a routable cell.",
                        net.name, sink.name
                    ));
                    continue;
                }
            };

            let sink_nets = endpoint_sink_nets(Some(driver_cell), sink_cell, sink, context.wires);
            if sink_nets.is_empty() {
                if should_skip_unmapped_sink(Some(driver_cell), sink_cell, sink) {
                    continue;
                }
                notes.push(format!(
                    "Net {} sink {}:{} has no route-sink mapping.",
                    net.name, sink.name, sink.pin
                ));
                continue;
            }

            let sink_wire_groups = if sink_requires_all_wires(sink_cell, sink) {
                sink_nets
                    .iter()
                    .copied()
                    .map(|wire| SmallVec::<[WireId; 1]>::from_buf([wire]))
                    .collect::<Vec<_>>()
            } else {
                vec![sink_nets]
            };

            let sink_guide = net.guide_tiles_for_sink(sink);
            let ordered_guide = OrderedGuide::new(sink_guide);
            let guide_distances = GuideDistances::new(arch, sink_guide);

            for sink_wires in sink_wire_groups {
                let spec = SinkRouteSpec {
                    net_index,
                    net_origin,
                    net_kind,
                    strict_clock_sink: net_kind == RouteNetKind::DedicatedClock
                        && sink_wires
                            .iter()
                            .all(|wire| is_clock_sink_wire_name(context.wires.resolve(*wire))),
                    ordered_guide: &ordered_guide,
                    guide_distances: &guide_distances,
                    roots: &roots,
                    tree_nodes: &tree_nodes,
                    sink_x: sink.x,
                    sink_y: sink.y,
                    sink_wires: sink_wires.as_slice(),
                };

                let Some((path, guide_mode)) = route_sink(
                    &context,
                    &occupied_route_sinks,
                    &occupied_route_nodes,
                    &spec,
                ) else {
                    notes.push(format!(
                        "Net {} could not find a Rust route from {}:{} to {}:{}.",
                        net.name, driver.name, driver.pin, sink.name, sink.pin
                    ));
                    continue;
                };

                guide_usage.record(guide_mode);
                reserve_route_sinks(&mut occupied_route_sinks, net_index, net_origin, &path.pips);
                reserve_route_nodes(
                    context.stitched_components,
                    &mut occupied_route_nodes,
                    net_index,
                    &path.nodes,
                );
                tree_nodes.extend(path.nodes.iter().copied());

                for pip in path.pips {
                    if used_pips.insert((pip.x, pip.y, pip.from, pip.to))
                        && let Some(materialized) = context.materialize_pip(pip, &net.name)
                    {
                        pips.push(materialized);
                    }
                }
            }
        }
    }

    notes.push(guide_usage.summary());
    Ok(DeviceRouteImage { pips, notes })
}

fn route_net_order_key(
    device: &DeviceDesign,
    index: &DeviceDesignIndex,
    net_index: usize,
) -> (u8, u8, usize, usize, usize) {
    let net = &device.nets[net_index];
    if !should_route_device_net(net) {
        return (2, 0, usize::MAX, usize::MAX, net_index);
    }

    let Some(driver) = net.driver.as_ref() else {
        return (1, 0, usize::MAX, usize::MAX, net_index);
    };

    let ResolvedRouteEndpoint::Cell(driver_cell) = resolve_route_endpoint(device, index, driver)
    else {
        return (1, 0, usize::MAX, usize::MAX, net_index);
    };

    let net_kind_rank = match classify_route_net_kind(driver_cell) {
        RouteNetKind::DedicatedClock => 0,
        RouteNetKind::Generic => 1,
    };
    let max_sink_distance = net
        .sinks
        .iter()
        .filter_map(|sink| match resolve_route_endpoint(device, index, sink) {
            ResolvedRouteEndpoint::Cell(sink_cell) => Some(tile_distance(
                driver_cell.x,
                driver_cell.y,
                sink_cell.x,
                sink_cell.y,
            )),
            _ => None,
        })
        .max()
        .unwrap_or(usize::MAX);

    (
        0,
        net_kind_rank,
        net.sinks.len(),
        max_sink_distance,
        net_index,
    )
}

#[derive(Default)]
struct GuideUsageStats {
    ordered: usize,
    strict: usize,
    relaxed: usize,
    fallback: usize,
    unguided: usize,
    dedicated_clock: usize,
}

impl GuideUsageStats {
    fn record(&mut self, mode: GuideRouteMode) {
        match mode {
            GuideRouteMode::Ordered => self.ordered += 1,
            GuideRouteMode::Strict => self.strict += 1,
            GuideRouteMode::Relaxed => self.relaxed += 1,
            GuideRouteMode::Fallback => self.fallback += 1,
            GuideRouteMode::Unguided => self.unguided += 1,
            GuideRouteMode::DedicatedClock => self.dedicated_clock += 1,
        }
    }

    fn summary(&self) -> String {
        format!(
            "Guide usage: ordered={}, strict={}, relaxed={}, fallback={}, unguided={}, dedicated_clock={}.",
            self.ordered,
            self.strict,
            self.relaxed,
            self.fallback,
            self.unguided,
            self.dedicated_clock
        )
    }
}

pub(super) struct RouteSinkContext<'a> {
    pub(super) arch: &'a Arch,
    pub(super) cil: &'a Cil,
    pub(super) graphs: &'a SiteRouteGraphs,
    pub(super) stitch_db: &'a TileStitchDb,
    pub(super) stitched_components: &'a StitchedComponentDb,
    pub(super) wires: &'a mut WireInterner,
}

impl RouteSinkContext<'_> {
    fn materialize_pip(&self, pip: RoutedPip, net_name: &str) -> Option<DeviceRoutePip> {
        let node = RouteNode::new(pip.x, pip.y, pip.to);
        let tile = route_context_for_node(self.arch, self.cil, &node)?;
        let graph = tile.graph(self.graphs)?;
        let arc = graph.arcs.get(pip.local_arc)?;
        Some(tile.pip(net_name.to_string(), pip.x, pip.y, arc, self.wires))
    }
}

#[derive(Debug, Clone)]
struct SinkRoutePath {
    nodes: Vec<RouteNode>,
    pips: Vec<RoutedPip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RouteNetKind {
    Generic,
    DedicatedClock,
}

pub(super) struct SinkRouteSpec<'a> {
    pub(super) net_index: usize,
    pub(super) net_origin: NetOrigin,
    pub(super) net_kind: RouteNetKind,
    pub(super) strict_clock_sink: bool,
    pub(super) ordered_guide: &'a OrderedGuide,
    pub(super) guide_distances: &'a GuideDistances,
    pub(super) roots: &'a [RouteNode],
    pub(super) tree_nodes: &'a HashSet<RouteNode>,
    pub(super) sink_x: usize,
    pub(super) sink_y: usize,
    pub(super) sink_wires: &'a [WireId],
}

fn ordered_start_nodes(spec: &SinkRouteSpec<'_>) -> Vec<RouteNode> {
    let mut nodes = spec.tree_nodes.iter().copied().collect::<Vec<_>>();
    if nodes.is_empty() {
        nodes.extend_from_slice(spec.roots);
    }
    nodes.sort_by_key(|node| {
        (
            tile_distance(node.x, node.y, spec.sink_x, spec.sink_y),
            node.x,
            node.y,
            node.wire,
        )
    });
    nodes.dedup();
    nodes
}

fn route_sink(
    context: &RouteSinkContext<'_>,
    occupied_route_sinks: &HashMap<(usize, usize, WireId), RouteSinkOwner>,
    occupied_route_nodes: &HashMap<RouteNode, RouteNodeOwner>,
    spec: &SinkRouteSpec<'_>,
) -> Option<(SinkRoutePath, GuideRouteMode)> {
    if spec.net_kind == RouteNetKind::DedicatedClock {
        return route_sink_with_policy(
            context,
            occupied_route_sinks,
            occupied_route_nodes,
            spec,
            None,
        )
        .map(|path| (path, GuideRouteMode::DedicatedClock));
    }

    if let Some(path) =
        route_sink_following_guide(context, occupied_route_sinks, occupied_route_nodes, spec)
    {
        return Some((path, GuideRouteMode::Ordered));
    }

    if spec.guide_distances.is_active() {
        for (max_guide_distance, mode) in [
            (Some(0usize), GuideRouteMode::Strict),
            (Some(1usize), GuideRouteMode::Relaxed),
            (Some(2usize), GuideRouteMode::Relaxed),
            (None, GuideRouteMode::Fallback),
        ] {
            if let Some(path) = route_sink_with_policy(
                context,
                occupied_route_sinks,
                occupied_route_nodes,
                spec,
                max_guide_distance,
            ) {
                return Some((path, mode));
            }
        }
        return None;
    }

    route_sink_with_policy(
        context,
        occupied_route_sinks,
        occupied_route_nodes,
        spec,
        None,
    )
    .map(|path| (path, GuideRouteMode::Unguided))
}

fn route_sink_following_guide(
    context: &RouteSinkContext<'_>,
    occupied_route_sinks: &HashMap<(usize, usize, WireId), RouteSinkOwner>,
    occupied_route_nodes: &HashMap<RouteNode, RouteNodeOwner>,
    spec: &SinkRouteSpec<'_>,
) -> Option<SinkRoutePath> {
    if !spec.ordered_guide.is_active()
        || spec.ordered_guide.len() < 2
        || spec.ordered_guide.last_tile() != Some((spec.sink_x, spec.sink_y))
    {
        return None;
    }

    let (frontier, best_cost) =
        seed_search(ordered_start_nodes(spec).into_iter().flat_map(|node| {
            spec.ordered_guide
                .indices_for_tile((node.x, node.y))
                .into_iter()
                .map(move |guide_index| {
                    let guided = GuidedRouteNode { node, guide_index };
                    (
                        guided,
                        spec.ordered_guide.remaining_steps(guide_index)
                            + tile_distance(node.x, node.y, spec.sink_x, spec.sink_y),
                        (guided.guide_index, guided.node.wire),
                    )
                })
        }));

    run_search(
        context,
        spec,
        frontier,
        best_cost,
        |guided| {
            guided.guide_index == spec.ordered_guide.last_index()
                && guided.node.x == spec.sink_x
                && guided.node.y == spec.sink_y
                && spec.sink_wires.contains(&guided.node.wire)
        },
        |guided| guided.node,
        |state, visit| {
            let availability = NeighborAvailability {
                stitched_components: context.stitched_components,
                occupied_route_sinks,
                occupied_route_nodes,
                net_index: spec.net_index,
                net_origin: spec.net_origin,
                tree_nodes: spec.tree_nodes,
            };
            for (neighbor, local_arc) in neighbors(
                context,
                &state.node.node,
                spec.net_kind,
                spec.strict_clock_sink,
            ) {
                if !neighbor_is_available(&availability, &state.node.node, &neighbor, local_arc) {
                    continue;
                }
                let Some(next_guide_index) = spec.ordered_guide.advance(
                    state.node.guide_index,
                    (state.node.node.x, state.node.node.y),
                    (neighbor.x, neighbor.y),
                ) else {
                    continue;
                };

                let next_node = GuidedRouteNode {
                    node: neighbor,
                    guide_index: next_guide_index,
                };
                let next_cost = state.cost
                    + route_transition_cost(context, spec, &state.node.node, &neighbor, local_arc);
                visit(
                    next_node,
                    local_arc,
                    next_cost,
                    next_cost
                        + spec.ordered_guide.remaining_steps(next_guide_index)
                        + tile_distance(neighbor.x, neighbor.y, spec.sink_x, spec.sink_y),
                    (next_node.guide_index, next_node.node.wire),
                );
            }
        },
    )
}

fn route_sink_with_policy(
    context: &RouteSinkContext<'_>,
    occupied_route_sinks: &HashMap<(usize, usize, WireId), RouteSinkOwner>,
    occupied_route_nodes: &HashMap<RouteNode, RouteNodeOwner>,
    spec: &SinkRouteSpec<'_>,
    max_guide_distance: Option<usize>,
) -> Option<SinkRoutePath> {
    let (frontier, best_cost) = seed_search(
        ordered_start_nodes(spec)
            .into_iter()
            .map(|node| (node, 0, node.wire)),
    );

    run_search(
        context,
        spec,
        frontier,
        best_cost,
        |node| {
            node.x == spec.sink_x && node.y == spec.sink_y && spec.sink_wires.contains(&node.wire)
        },
        |node| node,
        |state, visit| {
            let availability = NeighborAvailability {
                stitched_components: context.stitched_components,
                occupied_route_sinks,
                occupied_route_nodes,
                net_index: spec.net_index,
                net_origin: spec.net_origin,
                tree_nodes: spec.tree_nodes,
            };
            for (neighbor, local_arc) in
                neighbors(context, &state.node, spec.net_kind, spec.strict_clock_sink)
            {
                if !neighbor_is_available(&availability, &state.node, &neighbor, local_arc) {
                    continue;
                }
                if let Some(limit) = max_guide_distance
                    && (neighbor.x != state.node.x || neighbor.y != state.node.y)
                    && spec.guide_distances.distance(neighbor.x, neighbor.y) > limit
                {
                    continue;
                }

                let next_cost = state.cost
                    + route_transition_cost(context, spec, &state.node, &neighbor, local_arc)
                    + guide_penalty(&state.node, &neighbor, spec.guide_distances);
                let priority =
                    next_cost + route_heuristic(context, &neighbor, spec.sink_x, spec.sink_y);
                visit(neighbor, local_arc, next_cost, priority, neighbor.wire);
            }
        },
    )
}

fn seed_search<Node, Key>(
    starts: impl IntoIterator<Item = (Node, usize, Key)>,
) -> (Vec<SearchState<Node, Key>>, HashMap<Node, usize>)
where
    Node: Copy + Eq + Ord + std::hash::Hash,
    Key: Copy + Ord,
{
    let mut frontier = Vec::new();
    let mut best_cost = HashMap::new();
    for (node, priority, key) in starts {
        let order = frontier.len();
        frontier_heap_push(
            &mut frontier,
            SearchState {
                cost: 0,
                priority,
                order,
                key,
                node,
            },
        );
        best_cost.entry(node).or_insert(0);
    }
    (frontier, best_cost)
}

fn run_search<Node, Key>(
    context: &RouteSinkContext<'_>,
    spec: &SinkRouteSpec<'_>,
    mut frontier: Vec<SearchState<Node, Key>>,
    mut best_cost: HashMap<Node, usize>,
    is_goal: impl Fn(Node) -> bool,
    route_node_of: impl Fn(Node) -> RouteNode + Copy,
    mut expand: impl FnMut(
        &SearchState<Node, Key>,
        &mut dyn FnMut(Node, Option<usize>, usize, usize, Key),
    ),
) -> Option<SinkRoutePath>
where
    Node: Copy + Eq + Ord + std::hash::Hash,
    Key: Copy + Ord,
{
    let mut parent = HashMap::<Node, SearchParentStep<Node>>::new();
    let mut next_order = frontier.len();

    while let Some(state) = frontier_heap_pop(&mut frontier) {
        if is_goal(state.node) {
            return Some(reconstruct_search_path(
                context,
                state.node,
                route_node_of,
                |node| parent.get(node).map(|step| (step.previous, step.local_arc)),
            ));
        }

        let Some(current_best) = best_cost.get(&state.node).copied() else {
            continue;
        };
        if state.cost > current_best {
            continue;
        }

        expand(&state, &mut |node, local_arc, cost, priority, key| {
            if cost >= *best_cost.get(&node).unwrap_or(&usize::MAX) {
                return;
            }

            let joins_existing_tree = {
                let neighbor = route_node_of(node);
                spec.tree_nodes.contains(&neighbor) && !spec.roots.contains(&neighbor)
            };
            best_cost.insert(node, cost);
            parent.insert(
                node,
                SearchParentStep {
                    previous: (!joins_existing_tree).then_some(state.node),
                    local_arc: if joins_existing_tree { None } else { local_arc },
                },
            );
            frontier_heap_push(
                &mut frontier,
                SearchState {
                    cost,
                    priority,
                    order: next_order,
                    key,
                    node,
                },
            );
            next_order += 1;
        });
    }

    None
}

fn reconstruct_search_path<Node: Copy>(
    context: &RouteSinkContext<'_>,
    mut current: Node,
    route_node_of: impl Fn(Node) -> RouteNode,
    parent_step_of: impl Fn(&Node) -> Option<(Option<Node>, Option<usize>)>,
) -> SinkRoutePath {
    let mut reversed = Vec::new();
    let mut reversed_nodes = vec![route_node_of(current)];
    while let Some((previous, local_arc)) = parent_step_of(&current) {
        let Some(previous) = previous else {
            break;
        };
        let current_node = route_node_of(current);
        if let Some(arc_index) = local_arc
            && let Some(tile) = route_context_for_node(context.arch, context.cil, &current_node)
            && let Some(graph) = tile.graph(context.graphs)
            && let Some(arc) = graph.arcs.get(arc_index)
        {
            reversed.push(RoutedPip {
                x: current_node.x,
                y: current_node.y,
                from: arc.from,
                to: arc.to,
                local_arc: arc_index,
            });
        }
        current = previous;
        reversed_nodes.push(route_node_of(current));
    }
    reversed.reverse();
    reversed_nodes.reverse();
    SinkRoutePath {
        nodes: reversed_nodes,
        pips: reversed,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    use crate::{
        cil::load_cil,
        domain::NetOrigin,
        resource::{
            ResourceBundle, load_arch,
            routing::{
                StitchedComponentDb, build_stitched_components, load_site_route_graphs,
                load_tile_stitch_db,
            },
        },
    };

    use super::super::types::{RouteNode, SearchState, WireInterner};
    use super::{SinkRouteSpec, ordered_start_nodes};
    use crate::route::guide::GuideDistances;
    use crate::route::{
        guide::OrderedGuide,
        heap::{frontier_heap_pop, frontier_heap_push},
        occupancy::{RouteSinkOwner, route_node_is_available, route_sink_is_available},
    };

    #[test]
    fn ordered_guide_allows_long_span_progress_along_straight_runs() {
        let guide = OrderedGuide::new(&[
            (16, 31),
            (16, 30),
            (16, 29),
            (16, 28),
            (16, 27),
            (16, 26),
            (16, 25),
        ]);
        assert_eq!(guide.advance(0, (16, 31), (16, 25)), Some(6));
        assert_eq!(guide.advance(0, (16, 31), (16, 29)), Some(2));
        assert_eq!(guide.advance(2, (16, 29), (16, 25)), Some(6));
    }

    #[test]
    fn ordered_guide_rejects_skipping_across_turns() {
        let guide = OrderedGuide::new(&[(3, 3), (3, 4), (4, 4), (5, 4)]);
        assert_eq!(guide.advance(0, (3, 3), (5, 4)), None);
        assert_eq!(guide.advance(1, (3, 4), (5, 4)), Some(3));
    }

    #[test]
    fn route_sink_availability_keeps_configurable_sinks_single_owned() {
        let mut wires = WireInterner::default();
        let current_wire = wires.intern("CURRENT");
        let neighbor_wire = wires.intern("NEXT");
        let other_wire = wires.intern("OTHER");
        let current = RouteNode::new(7, 9, current_wire);
        let neighbor = RouteNode::new(7, 9, neighbor_wire);
        let mut occupied = HashMap::new();

        assert!(route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            Some(0),
        ));

        occupied.insert(
            (7, 9, neighbor_wire),
            RouteSinkOwner {
                net_index: 1,
                origin: NetOrigin::Logical,
                from: current_wire,
            },
        );
        assert!(route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            Some(0),
        ));

        occupied.insert(
            (7, 9, neighbor_wire),
            RouteSinkOwner {
                net_index: 2,
                origin: NetOrigin::Logical,
                from: other_wire,
            },
        );
        assert!(!route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            Some(0),
        ));
        assert!(route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            None,
        ));
    }

    #[test]
    fn synthetic_gclk_owner_can_share_same_programmable_sink_arc() {
        let mut wires = WireInterner::default();
        let current_wire = wires.intern("GCLK_PW");
        let neighbor_wire = wires.intern("GCLK");
        let current = RouteNode::new(34, 27, current_wire);
        let neighbor = RouteNode::new(34, 27, neighbor_wire);
        let occupied = HashMap::from([(
            (34, 27, neighbor_wire),
            RouteSinkOwner {
                net_index: 0,
                origin: NetOrigin::SyntheticGclk,
                from: current_wire,
            },
        )]);

        assert!(route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            Some(0),
        ));
        assert!(route_sink_is_available(
            &occupied,
            2,
            NetOrigin::SyntheticGclk,
            &current,
            &neighbor,
            Some(0),
        ));
    }

    #[test]
    fn synthetic_gclk_sharing_still_requires_matching_source_wire() {
        let mut wires = WireInterner::default();
        let source_wire = wires.intern("GCLK_PW");
        let other_source_wire = wires.intern("OTHER_PW");
        let neighbor_wire = wires.intern("GCLK");
        let current = RouteNode::new(34, 27, other_source_wire);
        let neighbor = RouteNode::new(34, 27, neighbor_wire);
        let occupied = HashMap::from([(
            (34, 27, neighbor_wire),
            RouteSinkOwner {
                net_index: 0,
                origin: NetOrigin::SyntheticGclk,
                from: source_wire,
            },
        )]);

        assert!(!route_sink_is_available(
            &occupied,
            1,
            NetOrigin::Logical,
            &current,
            &neighbor,
            Some(0),
        ));
    }

    #[test]
    fn all_route_nodes_are_reserved_across_nets() {
        let mut wires = WireInterner::default();
        let track = RouteNode::new(5, 7, wires.intern("H6W2"));
        let site_sink = RouteNode::new(5, 7, wires.intern("S0_F_B1"));
        let occupied = HashMap::from([(track, 0usize), (site_sink, 0usize)]);
        let tree_nodes = HashSet::new();
        let stitched_components = StitchedComponentDb::default();

        assert!(!route_node_is_available(
            &stitched_components,
            &occupied,
            1,
            &track,
            &tree_nodes,
        ));
        assert!(route_node_is_available(
            &stitched_components,
            &occupied,
            0,
            &track,
            &tree_nodes,
        ));
        assert!(!route_node_is_available(
            &stitched_components,
            &occupied,
            1,
            &site_sink,
            &tree_nodes,
        ));
    }

    #[test]
    fn real_arch_stitched_component_occupancy_blocks_shared_llh_track_across_tiles() {
        let Some(bundle) =
            ResourceBundle::discover_from(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))).ok()
        else {
            return;
        };
        let arch_path = bundle.root.join("fdp3p7_arch.xml");
        if !arch_path.exists() {
            return;
        }

        let arch = load_arch(&arch_path).expect("load arch");
        let mut wires = WireInterner::default();
        let db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");
        let components = build_stitched_components(&db, &arch, &wires);

        let upper = RouteNode::new(4, 53, wires.intern("RIGHT_LLH3"));
        let lower = RouteNode::new(4, 5, wires.intern("LLH0"));
        let tree_nodes = HashSet::new();

        assert_eq!(
            components.occupancy_key(&upper),
            components.occupancy_key(&lower)
        );

        let occupied = HashMap::from([(components.occupancy_key(&upper), 0usize)]);
        assert!(!route_node_is_available(
            &components,
            &occupied,
            1,
            &lower,
            &tree_nodes,
        ));
        assert!(route_node_is_available(
            &components,
            &occupied,
            0,
            &lower,
            &tree_nodes,
        ));
    }

    #[test]
    fn frontier_heap_has_stable_equal_cost_pop_order() {
        let mut wires = WireInterner::default();
        let mut heap = Vec::new();
        for node in 0..8 {
            frontier_heap_push(
                &mut heap,
                SearchState {
                    cost: 1,
                    priority: 1,
                    order: 0,
                    key: wires.intern_indexed("N", node),
                    node: RouteNode::new(0, 0, wires.intern_indexed("N", node)),
                },
            );
        }

        let mut popped = Vec::new();
        while let Some(state) = frontier_heap_pop(&mut heap) {
            popped.push(state.node.wire);
        }

        let expected = [0usize, 1, 3, 7, 6, 5, 4, 2]
            .into_iter()
            .map(|index| wires.intern_indexed("N", index))
            .collect::<Vec<_>>();
        assert_eq!(popped, expected);
    }

    #[test]
    fn ordered_start_nodes_prioritize_existing_tree_frontier() {
        let mut wires = WireInterner::default();
        let root = RouteNode::new(0, 0, wires.intern("ROOT"));
        let near_tree = RouteNode::new(4, 4, wires.intern("NEAR"));
        let far_tree = RouteNode::new(1, 1, wires.intern("FAR"));
        let ordered_guide = OrderedGuide::new(&[]);
        let guide_distances = GuideDistances::new(&crate::resource::Arch::default(), &[]);
        let tree_nodes = HashSet::from([root, far_tree, near_tree]);
        let sink_wires = [wires.intern("SINK")];
        let spec = SinkRouteSpec {
            net_index: 0,
            net_origin: NetOrigin::Logical,
            net_kind: super::RouteNetKind::Generic,
            strict_clock_sink: false,
            ordered_guide: &ordered_guide,
            guide_distances: &guide_distances,
            roots: &[root],
            tree_nodes: &tree_nodes,
            sink_x: 5,
            sink_y: 5,
            sink_wires: &sink_wires,
        };

        assert_eq!(ordered_start_nodes(&spec), vec![near_tree, far_tree, root]);
    }

    #[test]
    fn dedicated_clock_search_reaches_real_arch_clock_sink() {
        let Some(bundle) =
            ResourceBundle::discover_from(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))).ok()
        else {
            return;
        };
        let arch_path = bundle.root.join("fdp3p7_arch.xml");
        let cil_path = bundle.root.join("fdp3p7_cil.xml");
        if !arch_path.exists() || !cil_path.exists() {
            return;
        }

        let arch = load_arch(&arch_path).expect("load arch");
        let cil = load_cil(&cil_path).expect("load cil");
        let mut wires = WireInterner::default();
        let graphs = load_site_route_graphs(&arch_path, &cil, &mut wires).expect("load graphs");
        let stitch_db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");
        let stitched_components = build_stitched_components(&stitch_db, &arch, &wires);
        let root = RouteNode::new(34, 27, wires.intern("CLKB_GCLK1_PW"));
        let sink_wire = wires.intern("S0_CLK_B");
        let ordered_guide = OrderedGuide::new(&[]);
        let guide_distances = GuideDistances::new(&arch, &[]);
        let tree_nodes = HashSet::from([root]);
        let sink_wires = [sink_wire];
        let spec = SinkRouteSpec {
            net_index: 0,
            net_origin: NetOrigin::Logical,
            net_kind: super::RouteNetKind::DedicatedClock,
            strict_clock_sink: true,
            ordered_guide: &ordered_guide,
            guide_distances: &guide_distances,
            roots: &[root],
            tree_nodes: &tree_nodes,
            sink_x: 3,
            sink_y: 31,
            sink_wires: &sink_wires,
        };
        let context = super::RouteSinkContext {
            arch: &arch,
            cil: &cil,
            graphs: &graphs,
            stitch_db: &stitch_db,
            stitched_components: &stitched_components,
            wires: &mut wires,
        };

        let path =
            super::route_sink_with_policy(&context, &HashMap::new(), &HashMap::new(), &spec, None);
        let path = path.expect("dedicated clock route");
        let wires_on_path = path
            .nodes
            .iter()
            .map(|node| context.wires.resolve(node.wire).to_string())
            .collect::<Vec<_>>();
        assert!(wires_on_path.iter().any(|wire| wire == "CLKC_VGCLK1"));
        assert!(wires_on_path.iter().any(|wire| wire == "CLKV_GCLK_BUFR1"));
        assert!(!wires_on_path.iter().any(|wire| wire.starts_with("BRAM_")));
    }
}
