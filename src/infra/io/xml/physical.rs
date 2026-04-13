mod bindings;
mod nets;
mod ports;

use crate::ir::{ClusterId, Design};
use std::collections::{BTreeMap, BTreeSet};

use super::writer::{PhysicalDesignView, PhysicalInstance};
use super::writer::{SliceCellBinding, XmlWriteContext};
use crate::domain::ClusterKind;
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
        let (block_ram_instances, block_ram_bindings) =
            block_ram_instances_and_bindings(design, &index);

        let port_bindings = build_port_bindings(design, context);
        let mut used_modules = BTreeSet::from(["slice"]);
        let mut instances = slice_instances;
        if !block_ram_instances.is_empty() {
            used_modules.insert("blockram");
            instances.extend(block_ram_instances);
        }

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

        let nets = build_physical_nets(
            design,
            &index,
            &cell_bindings,
            &block_ram_bindings,
            &port_bindings,
        );
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
        if cluster.kind != ClusterKind::Logic {
            continue;
        }
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

fn block_ram_instances_and_bindings(
    design: &Design,
    index: &crate::ir::DesignIndex<'_>,
) -> (Vec<PhysicalInstance>, BTreeMap<String, String>) {
    let mut instances = Vec::new();
    let mut bindings = BTreeMap::<String, String>::new();
    let mut block_ram_index = 0usize;

    for (cluster_index, cluster) in design.clusters.iter().enumerate() {
        if cluster.kind != ClusterKind::BlockRam {
            continue;
        }
        let cluster_id = ClusterId::new(cluster_index);
        let members = index
            .cluster_members(cluster_id)
            .iter()
            .copied()
            .filter_map(|cell_id| {
                let cell = index.cell(design, cell_id);
                cell.is_block_ram().then_some(cell)
            })
            .collect::<Vec<_>>();
        if members.is_empty() {
            continue;
        }

        let instance_name = format!("iBram__{block_ram_index}__");
        block_ram_index += 1;
        for cell in &members {
            bindings.insert(cell.name.clone(), instance_name.clone());
        }
        let cell = members[0];
        instances.push(PhysicalInstance {
            name: instance_name,
            module_ref: "blockram",
            position: instance_position(design, cluster.x, cluster.y, cluster.z),
            configs: cell
                .properties
                .iter()
                .map(|property| (property.key.clone(), property.value.clone()))
                .collect(),
        });
    }

    (instances, bindings)
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

#[cfg(test)]
mod tests {
    use super::super::writer::{XmlWriteContext, save_design_xml};
    use crate::{
        domain::{CellKind, ClusterKind},
        ir::{Cell, Cluster, Design, Endpoint, Net, Port, Property},
    };

    #[test]
    fn physical_xml_emits_blockram_instances_and_endpoints() {
        let mut ram = Cell::new("ram0", CellKind::BlockRam, "BLOCKRAM_1")
            .in_cluster("bram_0000")
            .with_input("CKA", "clk")
            .with_input("DI0", "din")
            .with_output("DO0", "dout");
        ram.properties.push(Property::new("PORTA_ATTR", "512X8"));
        ram.properties
            .push(Property::new("INIT_00", "0123456789ABCDEF"));

        let design = Design {
            name: "bram_physical".to_string(),
            stage: "routed".to_string(),
            ports: vec![Port::input("clk"), Port::input("din"), Port::output("q")],
            cells: vec![ram],
            nets: vec![
                Net::new("clk")
                    .with_driver(Endpoint::port("clk", "clk"))
                    .with_sink(Endpoint::cell("ram0", "CKA")),
                Net::new("din")
                    .with_driver(Endpoint::port("din", "din"))
                    .with_sink(Endpoint::cell("ram0", "DI0")),
                Net::new("dout")
                    .with_driver(Endpoint::cell("ram0", "DO0"))
                    .with_sink(Endpoint::port("q", "q")),
            ],
            clusters: vec![
                Cluster::new("bram_0000", ClusterKind::BlockRam)
                    .with_member("ram0")
                    .at_slot(14, 54, 0),
            ],
            ..Design::default()
        };

        let xml = save_design_xml(&design, &XmlWriteContext::default()).expect("physical xml");
        let doc = roxmltree::Document::parse(&xml).expect("parse xml");

        assert!(
            doc.descendants()
                .any(|node| node.has_tag_name("module")
                    && node.attribute("name") == Some("blockram"))
        );
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("instance")
                && node.attribute("name") == Some("iBram__0__")
                && node.attribute("moduleRef") == Some("blockram")
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("config")
                && node.attribute("name") == Some("PORTA_ATTR")
                && node.attribute("value") == Some("512X8")
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("portRef")
                && node.attribute("name") == Some("CKA")
                && node.attribute("instanceRef") == Some("iBram__0__")
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("portRef")
                && node.attribute("name") == Some("DINA0")
                && node.attribute("instanceRef") == Some("iBram__0__")
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("portRef")
                && node.attribute("name") == Some("DOUTA0")
                && node.attribute("instanceRef") == Some("iBram__0__")
        }));
    }

    #[test]
    fn physical_xml_keeps_unused_constrained_input_iob_disabled() {
        let design = Design {
            name: "unused_input".to_string(),
            stage: "placed".to_string(),
            ports: vec![
                Port::input("din").at_site(1, 1, 0),
                Port::input("uart_rx").at_site(2, 1, 0),
                Port::output("dout").at_site(3, 1, 0),
            ],
            nets: vec![
                Net::new("data")
                    .with_driver(Endpoint::port("din", "din"))
                    .with_sink(Endpoint::port("dout", "dout")),
            ],
            ..Design::default()
        };

        let xml = save_design_xml(&design, &XmlWriteContext::default()).expect("physical xml");
        let doc = roxmltree::Document::parse(&xml).expect("parse xml");

        assert!(doc.descendants().any(|node| {
            node.has_tag_name("instance")
                && node.attribute("name") == Some("uart_rx")
                && node.attribute("moduleRef") == Some("iob")
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("instance")
                && node.attribute("name") == Some("uart_rx")
                && node.children().any(|child| {
                    child.has_tag_name("config")
                        && child.attribute("name") == Some("IMUX")
                        && child.attribute("value") == Some("#OFF")
                })
        }));
        assert!(doc.descendants().any(|node| {
            node.has_tag_name("instance")
                && node.attribute("name") == Some("uart_rx")
                && node.children().any(|child| {
                    child.has_tag_name("config")
                        && child.attribute("name") == Some("IOATTRBOX")
                        && child.attribute("value") == Some("#OFF")
                })
        }));
        assert!(
            !doc.descendants().any(|node| {
                node.has_tag_name("net") && node.attribute("name") == Some("uart_rx")
            })
        );
    }
}
