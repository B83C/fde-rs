mod bindings;
mod nets;
mod ports;

use crate::ir::{ClusterId, Design};
use std::collections::{BTreeMap, BTreeSet};

use super::writer::{PhysicalDesignView, PhysicalInstance};
use super::writer::{SliceCellBinding, XmlWriteContext};
use bindings::{assign_cluster_cells, build_slice_configs};
use nets::build_physical_nets;
use ports::{build_pad_configs, build_port_bindings};

impl PhysicalDesignView {
    pub(super) fn build(design: &Design, context: &XmlWriteContext<'_>) -> Option<Self> {
        if !supports_physical_view(design) {
            return None;
        }
        if has_unclustered_non_constant_cells(design) {
            return None;
        }

        let index = design.index();
        let (slice_instances, cell_bindings) = slice_instances_and_bindings(design, &index);

        let port_bindings = build_port_bindings(design, context);
        let mut used_modules = BTreeSet::from(["slice"]);
        let mut instances = slice_instances;

        for instance in non_clock_port_instances(design, &port_bindings) {
            used_modules.insert(instance.module_ref);
            instances.push(instance);
        }

        for instance in gclk_instances(design, &port_bindings) {
            used_modules.insert("gclk");
            instances.push(instance);
        }

        for instance in clock_pad_instances(design, &port_bindings) {
            used_modules.insert("gclkiob");
            instances.push(instance);
        }

        if instances.is_empty() {
            return None;
        }

        let nets = build_physical_nets(design, &index, &cell_bindings, &port_bindings);
        if nets.is_empty() {
            return None;
        }
        used_modules.extend(modules_used_by_port_bindings(&port_bindings));

        Some(Self {
            instances,
            nets,
            used_modules,
            include_capacitance: matches!(design.stage.as_str(), "placed" | "routed" | "timed"),
        })
    }
}

fn supports_physical_view(design: &Design) -> bool {
    matches!(
        design.stage.as_str(),
        "packed" | "placed" | "routed" | "timed"
    )
}

fn has_unclustered_non_constant_cells(design: &Design) -> bool {
    design
        .cells
        .iter()
        .any(|cell| !cell.is_constant_source() && cell.cluster.is_none())
}

fn slice_instances_and_bindings(
    design: &Design,
    index: &crate::ir::DesignIndex<'_>,
) -> (
    Vec<PhysicalInstance>,
    BTreeMap<String, (String, SliceCellBinding)>,
) {
    let mut slice_instances = Vec::new();
    let mut cell_bindings = BTreeMap::<String, (String, SliceCellBinding)>::new();
    for (cluster_index, cluster) in design.clusters.iter().enumerate() {
        let instance_name = format!("iSlice__{cluster_index}__");
        let position = instance_position(design, cluster.x, cluster.y, cluster.z);
        let cells = assign_cluster_cells(design, index, ClusterId::new(cluster_index));
        if cells.is_empty() {
            continue;
        }
        for (cell_name, binding) in &cells {
            cell_bindings.insert(cell_name.clone(), (instance_name.clone(), *binding));
        }
        let configs = build_slice_configs(design, &cells);
        slice_instances.push(PhysicalInstance {
            name: instance_name.clone(),
            module_ref: "slice",
            position,
            configs,
        });
    }
    (slice_instances, cell_bindings)
}

fn instance_position(
    design: &Design,
    x: Option<usize>,
    y: Option<usize>,
    z: Option<usize>,
) -> Option<(usize, usize, usize)> {
    (design.stage != "packed")
        .then(|| x.zip(y).map(|(x, y)| (x, y, z.unwrap_or(0))))
        .flatten()
}

fn non_clock_port_instances(
    design: &Design,
    port_bindings: &[super::writer::PortInstanceBinding],
) -> Vec<PhysicalInstance> {
    let mut instances = port_bindings
        .iter()
        .filter(|binding| binding.pad_module_ref != "gclkiob")
        .map(|binding| PhysicalInstance {
            name: binding.pad_instance_name.clone(),
            module_ref: binding.pad_module_ref,
            position: instance_position(
                design,
                binding.pad_position.map(|(x, _, _)| x),
                binding.pad_position.map(|(_, y, _)| y),
                binding.pad_position.map(|(_, _, z)| z),
            ),
            configs: build_pad_configs(binding),
        })
        .collect::<Vec<_>>();
    instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
    instances
}

fn gclk_instances(
    design: &Design,
    port_bindings: &[super::writer::PortInstanceBinding],
) -> Vec<PhysicalInstance> {
    let mut instances = port_bindings
        .iter()
        .filter_map(|binding| {
            binding
                .gclk_instance_name
                .as_ref()
                .map(|name| PhysicalInstance {
                    name: name.clone(),
                    module_ref: "gclk",
                    position: instance_position(
                        design,
                        binding.gclk_position.map(|(x, _, _)| x),
                        binding.gclk_position.map(|(_, y, _)| y),
                        binding.gclk_position.map(|(_, _, z)| z),
                    ),
                    configs: super::writer::default_configs(super::writer::GCLK_DEFAULT_CONFIGS),
                })
        })
        .collect::<Vec<_>>();
    instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
    instances
}

fn clock_pad_instances(
    design: &Design,
    port_bindings: &[super::writer::PortInstanceBinding],
) -> Vec<PhysicalInstance> {
    let mut instances = port_bindings
        .iter()
        .filter(|binding| binding.pad_module_ref == "gclkiob")
        .map(|binding| PhysicalInstance {
            name: binding.pad_instance_name.clone(),
            module_ref: "gclkiob",
            position: instance_position(
                design,
                binding.pad_position.map(|(x, _, _)| x),
                binding.pad_position.map(|(_, y, _)| y),
                binding.pad_position.map(|(_, _, z)| z),
            ),
            configs: super::writer::default_configs(super::writer::GCLKIOB_DEFAULT_CONFIGS),
        })
        .collect::<Vec<_>>();
    instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
    instances
}

fn modules_used_by_port_bindings(
    port_bindings: &[super::writer::PortInstanceBinding],
) -> BTreeSet<&'static str> {
    let mut used_modules = BTreeSet::new();
    for binding in port_bindings {
        used_modules.insert(binding.pad_module_ref);
        if binding.gclk_instance_name.is_some() {
            used_modules.insert("gclk");
        }
    }
    used_modules
}
