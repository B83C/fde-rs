use crate::route::{
    types::RouteNode,
    wire::{
        WireBounds, route_node_base_cost, route_node_class_for_wire, tile_distance,
        wire_bounds_for_wire,
    },
};

use super::{
    policy::node_has_successors,
    router::{RouteSinkContext, SinkRouteSpec},
};

pub(super) fn route_transition_cost(
    context: &RouteSinkContext<'_>,
    _spec: &SinkRouteSpec<'_>,
    _current: &RouteNode,
    neighbor: &RouteNode,
    local_arc: Option<usize>,
) -> usize {
    if local_arc.is_none() {
        0
    } else {
        route_node_cost(context, neighbor)
    }
}

pub(super) fn route_heuristic(
    context: &RouteSinkContext<'_>,
    node: &RouteNode,
    sink_x: usize,
    sink_y: usize,
) -> usize {
    let Some(bounds) = context.stitched_components.bounds(node) else {
        if let Some(bounds) =
            wire_bounds_for_wire(context.arch, node.x, node.y, context.wires, node.wire)
        {
            return axis_distance(sink_x, bounds.min_x, bounds.max_x)
                + axis_distance(sink_y, bounds.min_y, bounds.max_y);
        }
        return tile_distance(node.x, node.y, sink_x, sink_y);
    };

    axis_distance(sink_x, bounds.min_x, bounds.max_x)
        + axis_distance(sink_y, bounds.min_y, bounds.max_y)
}

fn route_node_cost(context: &RouteSinkContext<'_>, node: &RouteNode) -> usize {
    let bounds = context
        .stitched_components
        .bounds(node)
        .map(|bounds| WireBounds {
            min_x: bounds.min_x,
            max_x: bounds.max_x,
            min_y: bounds.min_y,
            max_y: bounds.max_y,
        })
        .or_else(|| wire_bounds_for_wire(context.arch, node.x, node.y, context.wires, node.wire));
    let class = route_node_class_for_wire(
        context.wires,
        node.wire,
        bounds,
        node_has_successors(context, node),
    );
    route_node_base_cost(class)
}

fn axis_distance(value: usize, min: usize, max: usize) -> usize {
    if value < min {
        min - value
    } else {
        value.saturating_sub(max)
    }
}
