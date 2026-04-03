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
        if !matches!(
            design.stage.as_str(),
            "packed" | "placed" | "routed" | "timed"
        ) {
            return None;
        }
        if design
            .cells
            .iter()
            .any(|cell| !cell.is_constant_source() && cell.cluster.is_none())
        {
            return None;
        }

        let index = design.index();
        let mut slice_instances = Vec::new();
        let mut cell_bindings = BTreeMap::<String, (String, SliceCellBinding)>::new();
        for (cluster_index, cluster) in design.clusters.iter().enumerate() {
            let instance_name = format!("iSlice__{cluster_index}__");
            let position = (design.stage != "packed")
                .then(|| {
                    cluster
                        .x
                        .zip(cluster.y)
                        .map(|(x, y)| (x, y, cluster.z.unwrap_or(0)))
                })
                .flatten();
            let cells = assign_cluster_cells(design, &index, ClusterId::new(cluster_index));
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

        let port_bindings = build_port_bindings(design, context);
        let mut used_modules = BTreeSet::from(["slice"]);
        let mut instances = slice_instances;

        let mut non_clock_port_instances = port_bindings
            .iter()
            .filter(|binding| binding.pad_module_ref != "gclkiob")
            .map(|binding| PhysicalInstance {
                name: binding.pad_instance_name.clone(),
                module_ref: binding.pad_module_ref,
                position: (design.stage != "packed")
                    .then_some(binding.pad_position)
                    .flatten(),
                configs: build_pad_configs(binding),
            })
            .collect::<Vec<_>>();
        non_clock_port_instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
        for instance in non_clock_port_instances {
            used_modules.insert(instance.module_ref);
            instances.push(instance);
        }

        let mut gclk_instances = port_bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .gclk_instance_name
                    .as_ref()
                    .map(|name| PhysicalInstance {
                        name: name.clone(),
                        module_ref: "gclk",
                        position: (design.stage != "packed")
                            .then_some(binding.gclk_position)
                            .flatten(),
                        configs: super::writer::default_configs(
                            super::writer::GCLK_DEFAULT_CONFIGS,
                        ),
                    })
            })
            .collect::<Vec<_>>();
        gclk_instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
        for instance in gclk_instances {
            used_modules.insert("gclk");
            instances.push(instance);
        }

        let mut clock_pad_instances = port_bindings
            .iter()
            .filter(|binding| binding.pad_module_ref == "gclkiob")
            .map(|binding| PhysicalInstance {
                name: binding.pad_instance_name.clone(),
                module_ref: "gclkiob",
                position: (design.stage != "packed")
                    .then_some(binding.pad_position)
                    .flatten(),
                configs: super::writer::default_configs(super::writer::GCLKIOB_DEFAULT_CONFIGS),
            })
            .collect::<Vec<_>>();
        clock_pad_instances.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
        for instance in clock_pad_instances {
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
        for binding in &port_bindings {
            used_modules.insert(binding.pad_module_ref);
            if binding.gclk_instance_name.is_some() {
                used_modules.insert("gclk");
            }
        }

        Some(Self {
            instances,
            nets,
            used_modules,
            include_capacitance: matches!(design.stage.as_str(), "placed" | "routed" | "timed"),
        })
    }
}
