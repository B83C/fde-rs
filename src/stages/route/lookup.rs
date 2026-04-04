use crate::{cil::Cil, resource::Arch};

use super::types::{
    DeviceRoutePip, RouteBit, RouteNode, SiteRouteArc, SiteRouteGraph, SiteRouteGraphs,
    WireInterner,
};

pub(crate) struct TileRouteContext<'a> {
    pub(crate) tile_name: &'a str,
    pub(crate) tile_type: &'a str,
    pub(crate) site_name: &'a str,
    pub(crate) site_type: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedTileRouteContext<'a> {
    pub(crate) tile_name: &'a str,
    pub(crate) tile_type: &'a str,
    pub(crate) site_name: &'a str,
    pub(crate) site_type: &'a str,
    pub(crate) graph: Option<&'a SiteRouteGraph>,
}

pub(crate) struct TileRouteCache<'a> {
    width: usize,
    height: usize,
    entries: Vec<Option<CachedTileRouteContext<'a>>>,
}

impl<'a> TileRouteContext<'a> {
    pub(crate) fn graph<'g>(&self, graphs: &'g SiteRouteGraphs) -> Option<&'g SiteRouteGraph> {
        graphs.get(self.site_type)
    }

    pub(crate) fn pip(
        &self,
        net_name: String,
        x: usize,
        y: usize,
        arc: &SiteRouteArc,
        wires: &WireInterner,
    ) -> DeviceRoutePip {
        DeviceRoutePip {
            net_name,
            tile_name: self.tile_name.to_string(),
            tile_type: self.tile_type.to_string(),
            site_name: self.site_name.to_string(),
            site_type: self.site_type.to_string(),
            x,
            y,
            from_net: wires.resolve(arc.from).to_string(),
            to_net: wires.resolve(arc.to).to_string(),
            bits: arc
                .bits
                .iter()
                .map(|bit| RouteBit {
                    basic_cell: arc.basic_cell.clone(),
                    sram_name: bit.sram_name.clone(),
                    value: bit.value,
                })
                .collect(),
        }
    }
}

impl<'a> CachedTileRouteContext<'a> {
    pub(crate) fn pip(
        &self,
        net_name: String,
        x: usize,
        y: usize,
        arc: &SiteRouteArc,
        wires: &WireInterner,
    ) -> DeviceRoutePip {
        DeviceRoutePip {
            net_name,
            tile_name: self.tile_name.to_string(),
            tile_type: self.tile_type.to_string(),
            site_name: self.site_name.to_string(),
            site_type: self.site_type.to_string(),
            x,
            y,
            from_net: wires.resolve(arc.from).to_string(),
            to_net: wires.resolve(arc.to).to_string(),
            bits: arc
                .bits
                .iter()
                .map(|bit| RouteBit {
                    basic_cell: arc.basic_cell.clone(),
                    sram_name: bit.sram_name.clone(),
                    value: bit.value,
                })
                .collect(),
        }
    }
}

impl<'a> TileRouteCache<'a> {
    pub(crate) fn build(arch: &'a Arch, cil: &'a Cil, graphs: &'a SiteRouteGraphs) -> Self {
        let mut entries = vec![None; arch.width.saturating_mul(arch.height).max(1)];
        for x in 0..arch.width {
            for y in 0..arch.height {
                let Some(tile) = arch.tile_at(x, y) else {
                    continue;
                };
                let Some(tile_def) = cil.tiles.get(tile.tile_type.as_str()) else {
                    continue;
                };
                let Some(transmission) = tile_def.transmissions.first() else {
                    continue;
                };
                let Some(site) = transmission.sites.first() else {
                    continue;
                };
                let index = y * arch.width + x;
                entries[index] = Some(CachedTileRouteContext {
                    tile_name: tile.name.as_str(),
                    tile_type: tile.tile_type.as_str(),
                    site_name: site.name.as_str(),
                    site_type: transmission.site_type.as_str(),
                    graph: graphs.get(transmission.site_type.as_str()),
                });
            }
        }

        Self {
            width: arch.width,
            height: arch.height,
            entries,
        }
    }

    pub(crate) fn for_node(&self, node: &RouteNode) -> Option<&CachedTileRouteContext<'a>> {
        if node.x >= self.width || node.y >= self.height {
            return None;
        }
        let index = node.y * self.width + node.x;
        self.entries.get(index)?.as_ref()
    }
}

pub(crate) fn route_context_for_node<'a>(
    arch: &'a Arch,
    cil: &'a Cil,
    node: &RouteNode,
) -> Option<TileRouteContext<'a>> {
    let tile = arch.tile_at(node.x, node.y)?;
    let tile_def = cil.tiles.get(tile.tile_type.as_str())?;
    let transmission = tile_def.transmissions.first()?;
    let site = transmission.sites.first()?;
    Some(TileRouteContext {
        tile_name: tile.name.as_str(),
        tile_type: tile.tile_type.as_str(),
        site_name: site.name.as_str(),
        site_type: transmission.site_type.as_str(),
    })
}
