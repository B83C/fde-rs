mod clock;
mod components;
mod site_graph;
mod stitch;
#[cfg(test)]
mod tests;

use arrayvec::ArrayString;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::{collections::HashMap, fmt::Write};

#[cfg(test)]
use clock::clock_spine_neighbors;
pub(crate) use components::build_stitched_components;
pub(crate) use site_graph::{load_site_route_defaults, load_site_route_graphs};
#[cfg(test)]
use stitch::neighbor_for_port;
pub(crate) use stitch::{load_tile_stitch_db, stitched_neighbors};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteBit {
    pub basic_cell: String,
    pub sram_name: String,
    pub value: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WireId(u32);

impl WireId {
    fn new(index: usize) -> Self {
        Self(index as u32)
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WireInterner {
    ids_by_name: HashMap<String, WireId>,
    names: Vec<String>,
}

impl WireInterner {
    pub(crate) fn intern(&mut self, name: &str) -> WireId {
        if let Some(id) = self.id(name) {
            return id;
        }
        self.intern_owned(name.to_string())
    }

    pub(crate) fn intern_owned(&mut self, name: String) -> WireId {
        if let Some(id) = self.id(&name) {
            return id;
        }
        let id = WireId::new(self.names.len());
        self.ids_by_name.insert(name.clone(), id);
        self.names.push(name);
        id
    }

    pub(crate) fn id(&self, name: &str) -> Option<WireId> {
        self.ids_by_name.get(name).copied()
    }

    pub(crate) fn intern_indexed(&mut self, prefix: &str, index: usize) -> WireId {
        self.intern_composite_indexed(prefix, "", index, "")
    }

    pub(crate) fn intern_composite_indexed(
        &mut self,
        first: &str,
        second: &str,
        index: usize,
        third: &str,
    ) -> WireId {
        if let Some(id) = self.id_composite_indexed(first, second, index, third) {
            return id;
        }

        let mut heap =
            String::with_capacity(first.len() + second.len() + third.len() + usize::BITS as usize);
        heap.push_str(first);
        heap.push_str(second);
        let _ = write!(&mut heap, "{index}");
        heap.push_str(third);
        self.intern_owned(heap)
    }

    pub(crate) fn id_indexed(&self, prefix: &str, index: usize) -> Option<WireId> {
        self.id_composite_indexed(prefix, "", index, "")
    }

    pub(crate) fn id_composite_indexed(
        &self,
        first: &str,
        second: &str,
        index: usize,
        third: &str,
    ) -> Option<WireId> {
        let mut stack = ArrayString::<48>::new();
        if stack.try_push_str(first).is_ok()
            && stack.try_push_str(second).is_ok()
            && write!(&mut stack, "{index}").is_ok()
            && stack.try_push_str(third).is_ok()
        {
            return self.id(stack.as_str());
        }

        let mut heap =
            String::with_capacity(first.len() + second.len() + third.len() + usize::BITS as usize);
        heap.push_str(first);
        heap.push_str(second);
        let _ = write!(&mut heap, "{index}");
        heap.push_str(third);
        self.id(&heap)
    }

    pub(crate) fn resolve(&self, id: WireId) -> &str {
        self.names
            .get(id.index())
            .map(String::as_str)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct RouteNode {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) wire: WireId,
}

impl RouteNode {
    pub(crate) fn new(x: usize, y: usize, wire: WireId) -> Self {
        Self { x, y, wire }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SiteRouteGraph {
    pub(crate) arcs: Vec<SiteRouteArc>,
    pub(crate) adjacency: HashMap<WireId, Vec<usize>>,
    pub(crate) default_bits: Vec<RouteBit>,
}

pub(crate) type SiteRouteGraphs = HashMap<String, SiteRouteGraph>;

#[derive(Debug, Clone)]
pub(crate) struct SiteRouteArc {
    pub(crate) from: WireId,
    pub(crate) to: WireId,
    pub(crate) basic_cell: String,
    pub(crate) bits: Vec<RouteBit>,
}

pub(crate) type SiteRouteDefaults = HashMap<String, Vec<RouteBit>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TileSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TilePortRef {
    pub(super) side: TileSide,
    pub(super) index: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TileWireStitch {
    pub(super) net_ports: HashMap<WireId, SmallVec<[TilePortRef; 2]>>,
    pub(super) port_nets: HashMap<(TileSide, usize), SmallVec<[WireId; 2]>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TileStitchDb {
    pub(super) tiles: HashMap<String, TileWireStitch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ComponentBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
}

impl ComponentBounds {
    pub(super) fn new(node: &RouteNode) -> Self {
        Self {
            min_x: node.x,
            max_x: node.x,
            min_y: node.y,
            max_y: node.y,
        }
    }

    pub(super) fn include(&mut self, node: &RouteNode) {
        self.min_x = self.min_x.min(node.x);
        self.max_x = self.max_x.max(node.x);
        self.min_y = self.min_y.min(node.y);
        self.max_y = self.max_y.max(node.y);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StitchedComponentDb {
    pub(super) bounds_by_node: HashMap<RouteNode, ComponentBounds>,
    pub(super) representative_by_node: HashMap<RouteNode, RouteNode>,
}

impl StitchedComponentDb {
    pub(crate) fn bounds(&self, node: &RouteNode) -> Option<ComponentBounds> {
        self.bounds_by_node.get(node).copied()
    }

    pub(crate) fn occupancy_key(&self, node: &RouteNode) -> RouteNode {
        self.representative_by_node
            .get(node)
            .copied()
            .unwrap_or(*node)
    }
}

#[derive(Debug, Clone)]
pub(super) struct TilePortDef {
    pub(super) name: String,
    pub(super) side: TileSide,
    pub(super) lsb: usize,
    pub(super) msb: usize,
}
