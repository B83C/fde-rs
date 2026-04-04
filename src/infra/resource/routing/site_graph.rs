use super::{
    RouteBit, SiteRouteArc, SiteRouteDefaults, SiteRouteGraph, SiteRouteGraphs, WireId,
    WireInterner,
};
use crate::cil::{Cil, ElementPath};
use anyhow::Result;
use roxmltree::Document;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use smallvec::SmallVec;
use std::{collections::BTreeMap, fs, path::Path};

pub(crate) fn load_site_route_graphs(
    path: &Path,
    cil: &Cil,
    wires: &mut WireInterner,
) -> Result<SiteRouteGraphs> {
    let xml = fs::read_to_string(path)?;
    let doc = Document::parse(&xml)?;
    let relevant = cil
        .tiles
        .values()
        .flat_map(|tile| {
            tile.transmissions
                .iter()
                .map(|transmission| transmission.site_type.clone())
        })
        .collect::<HashSet<_>>();
    let mut graphs = SiteRouteGraphs::default();

    for library in doc
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("library") && node.attribute("name") == Some("block"))
    {
        for cell in library.children().filter(|node| node.has_tag_name("cell")) {
            let Some(name) = cell.attribute("name") else {
                continue;
            };
            if !relevant.contains(name) {
                continue;
            }

            let mut instance_types = BTreeMap::<String, String>::new();
            let mut pin_to_nets = HashMap::<(String, String), Vec<WireId>>::default();
            if let Some(contents) = cell.children().find(|node| node.has_tag_name("contents")) {
                for instance in contents
                    .children()
                    .filter(|node| node.has_tag_name("instance"))
                {
                    let Some(instance_name) = instance.attribute("name") else {
                        continue;
                    };
                    let cell_ref = instance
                        .attribute("cellRef")
                        .unwrap_or_default()
                        .to_string();
                    instance_types.insert(instance_name.to_string(), cell_ref);
                }
                for net in contents.children().filter(|node| node.has_tag_name("net")) {
                    let Some(net_name) = net.attribute("name") else {
                        continue;
                    };
                    let wire = wires.intern(net_name);
                    for port_ref in net.children().filter(|node| node.has_tag_name("portRef")) {
                        let Some(instance_name) = port_ref.attribute("instanceRef") else {
                            continue;
                        };
                        let pin_name = port_ref.attribute("name").unwrap_or_default();
                        pin_to_nets
                            .entry((instance_name.to_string(), pin_name.to_string()))
                            .or_default()
                            .push(wire);
                    }
                }
            }

            let mut arcs = Vec::new();
            let mut seen = HashSet::default();
            for (instance_name, element_name) in &instance_types {
                let Some(element) = cil.elements.get(element_name) else {
                    continue;
                };
                for path in &element.paths {
                    let src_key = (instance_name.clone(), path.input.clone());
                    let dst_key = (instance_name.clone(), path.output.clone());
                    let Some(src_nets) = pin_to_nets.get(&src_key) else {
                        continue;
                    };
                    let Some(dst_nets) = pin_to_nets.get(&dst_key) else {
                        continue;
                    };
                    for &src in src_nets {
                        for &dst in dst_nets {
                            let key = (
                                src,
                                dst,
                                instance_name.clone(),
                                path.input.clone(),
                                path.output.clone(),
                            );
                            if !seen.insert(key) {
                                continue;
                            }
                            arcs.push(SiteRouteArc {
                                from: src,
                                to: dst,
                                basic_cell: instance_name.clone(),
                                bits: path_bits(path),
                            });
                        }
                    }
                }
            }

            let default_bits = instance_types
                .iter()
                .flat_map(|(instance_name, element_name)| {
                    cil.elements
                        .get(element_name)
                        .into_iter()
                        .flat_map(move |element| {
                            element.default_srams.iter().map(move |setting| RouteBit {
                                basic_cell: instance_name.clone(),
                                sram_name: setting.name.clone(),
                                value: setting.value,
                            })
                        })
                })
                .collect();

            let mut adjacency = vec![SmallVec::<[usize; 4]>::new(); wires.len()];
            for (index, arc) in arcs.iter().enumerate() {
                if should_skip_site_arc(name, arc, wires) {
                    continue;
                }
                let wire_index = arc.from.index();
                if wire_index >= adjacency.len() {
                    adjacency.resize(wire_index + 1, SmallVec::new());
                }
                adjacency[wire_index].push(index);
            }
            graphs.insert(
                name.to_string(),
                SiteRouteGraph {
                    arcs,
                    adjacency,
                    default_bits,
                },
            );
        }
    }

    Ok(graphs)
}

pub(crate) fn load_site_route_defaults(path: &Path, cil: &Cil) -> Result<SiteRouteDefaults> {
    let mut wires = WireInterner::default();
    let graphs = load_site_route_graphs(path, cil, &mut wires)?;
    Ok(graphs
        .into_iter()
        .map(|(site_type, graph)| (site_type, graph.default_bits))
        .collect())
}

fn path_bits(path: &ElementPath) -> Vec<RouteBit> {
    path.configuration
        .iter()
        .map(|setting| RouteBit {
            basic_cell: String::new(),
            sram_name: setting.name.clone(),
            value: setting.value,
        })
        .collect()
}

fn should_skip_site_arc(site_type: &str, arc: &SiteRouteArc, wires: &WireInterner) -> bool {
    if site_type != "GSB_LFT" {
        return false;
    }

    let from = wires.resolve(arc.from);
    let to = wires.resolve(arc.to);
    to.starts_with("LEFT_O") && from.starts_with("LEFT_H6") && from.contains("_BUF")
}
