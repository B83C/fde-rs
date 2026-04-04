use super::helpers::{attr, expand_bus_port_names, top_module_node};
use crate::ir::{
    Cell, CellKind, CellPin, Design, Endpoint, EndpointKind, Net, Port, PortDirection, Property,
};
use anyhow::{Result, anyhow, bail};
use roxmltree::Node;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct MappedXmlModule {
    type_name: String,
    port_directions: BTreeMap<String, PortDirection>,
}

#[derive(Debug, Clone)]
struct MappedXmlInstance {
    name: String,
    module_ref: String,
    keep: bool,
    cell: Cell,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum MappedXmlEndpointRef {
    Port { name: String },
    Cell { instance: String, pin: String },
}

struct RawMappedNets {
    net_names: Vec<String>,
    nets: Vec<Vec<MappedXmlEndpointRef>>,
    instance_nets: BTreeMap<String, BTreeMap<String, usize>>,
}

pub(super) fn load_fde_mapped_design_xml(root: Node<'_, '_>) -> Result<Design> {
    let module_node = top_module_node(root)?;
    let modules = mapped_modules(root);
    let contents = module_node
        .children()
        .find(|node| node.has_tag_name("contents"))
        .ok_or_else(|| anyhow!("FDE design is missing <contents>"))?;

    let ports = module_ports(module_node);
    let port_directions = port_directions(&ports);
    let instances = mapped_instances(contents, &modules)?;
    let instances_by_name = instances_by_name(&instances);

    if instances
        .iter()
        .any(|instance| is_physical_site_instance(&instance.module_ref))
    {
        bail!("direct physical packed/placed/routed FDE XML import is not implemented");
    }

    let raw_nets = raw_mapped_nets(contents);
    let grouped_indices = grouped_mapped_net_indices(&instances, &modules, &raw_nets);

    let mut cells = instances
        .iter()
        .filter(|instance| instance.keep)
        .map(|instance| (instance.name.clone(), instance.cell.clone()))
        .collect::<BTreeMap<_, _>>();
    let nets = build_mapped_nets(
        &grouped_indices,
        &raw_nets,
        &port_directions,
        &instances_by_name,
        &modules,
        &ports,
        &mut cells,
    );

    let mut design = Design {
        name: attr(&root, "name"),
        stage: "mapped".to_string(),
        metadata: crate::ir::Metadata {
            source_format: "fde-xml".to_string(),
            lut_size: 4,
            notes: vec!["Imported FDE mapped XML".to_string()],
            ..Default::default()
        },
        ports,
        cells: cells.into_values().collect(),
        nets,
        ..Design::default()
    };
    lower_mapped_constant_sources(&mut design);
    Ok(design)
}

fn module_ports(module_node: Node<'_, '_>) -> Vec<Port> {
    module_node
        .children()
        .filter(|node| node.has_tag_name("port"))
        .flat_map(mapped_ports)
        .collect()
}

fn port_directions(ports: &[Port]) -> BTreeMap<String, PortDirection> {
    ports
        .iter()
        .map(|port| (port.name.clone(), port.direction.clone()))
        .collect::<BTreeMap<_, _>>()
}

fn mapped_instances(
    contents: Node<'_, '_>,
    modules: &BTreeMap<String, MappedXmlModule>,
) -> Result<Vec<MappedXmlInstance>> {
    contents
        .children()
        .filter(|node| node.has_tag_name("instance"))
        .map(|node| mapped_instance(&node, modules))
        .collect::<Result<Vec<_>>>()
}

fn instances_by_name(instances: &[MappedXmlInstance]) -> BTreeMap<&str, &MappedXmlInstance> {
    instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>()
}

fn raw_mapped_nets(contents: Node<'_, '_>) -> RawMappedNets {
    let mut net_names = Vec::new();
    let mut nets = Vec::<Vec<MappedXmlEndpointRef>>::new();
    let mut instance_nets = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for net in contents.children().filter(|node| node.has_tag_name("net")) {
        let net_index = nets.len();
        net_names.push(attr(&net, "name"));
        let mut endpoints = Vec::new();
        for port_ref in net.children().filter(|node| node.has_tag_name("portRef")) {
            if let Some(instance_name) = port_ref.attribute("instanceRef") {
                let pin = attr(&port_ref, "name");
                instance_nets
                    .entry(instance_name.to_string())
                    .or_default()
                    .insert(pin.clone(), net_index);
                endpoints.push(MappedXmlEndpointRef::Cell {
                    instance: instance_name.to_string(),
                    pin,
                });
            } else {
                endpoints.push(MappedXmlEndpointRef::Port {
                    name: attr(&port_ref, "name"),
                });
            }
        }
        nets.push(endpoints);
    }
    RawMappedNets {
        net_names,
        nets,
        instance_nets,
    }
}

fn grouped_mapped_net_indices(
    instances: &[MappedXmlInstance],
    modules: &BTreeMap<String, MappedXmlModule>,
    raw_nets: &RawMappedNets,
) -> BTreeMap<usize, Vec<usize>> {
    let mut parent = (0..raw_nets.nets.len()).collect::<Vec<_>>();
    for instance in instances {
        if instance.keep {
            continue;
        }
        let Some(module) = modules.get(&instance.module_ref) else {
            continue;
        };
        let Some(pin_nets) = raw_nets.instance_nets.get(&instance.name) else {
            continue;
        };
        let input_nets = module
            .port_directions
            .iter()
            .filter(|(_, direction)| direction.is_input_like())
            .filter_map(|(pin, _)| pin_nets.get(pin).copied())
            .collect::<Vec<_>>();
        let output_nets = module
            .port_directions
            .iter()
            .filter(|(_, direction)| direction.is_output_like())
            .filter_map(|(pin, _)| pin_nets.get(pin).copied())
            .collect::<Vec<_>>();
        if input_nets.len() == 1 && output_nets.len() == 1 {
            disjoint_union(&mut parent, input_nets[0], output_nets[0]);
        }
    }

    let mut grouped_indices = BTreeMap::<usize, Vec<usize>>::new();
    for net_index in 0..raw_nets.nets.len() {
        let root_index = disjoint_find(&mut parent, net_index);
        grouped_indices
            .entry(root_index)
            .or_default()
            .push(net_index);
    }
    grouped_indices
}

fn build_mapped_nets(
    grouped_indices: &BTreeMap<usize, Vec<usize>>,
    raw_nets: &RawMappedNets,
    port_directions: &BTreeMap<String, PortDirection>,
    instances_by_name: &BTreeMap<&str, &MappedXmlInstance>,
    modules: &BTreeMap<String, MappedXmlModule>,
    ports: &[Port],
    cells: &mut BTreeMap<String, Cell>,
) -> Vec<Net> {
    let mut nets = Vec::new();
    for indices in grouped_indices.values() {
        let mut endpoints = BTreeSet::<MappedXmlEndpointRef>::new();
        for &net_index in indices {
            endpoints.extend(raw_nets.nets[net_index].iter().cloned());
        }

        let (drivers, sinks) =
            mapped_net_endpoints(&endpoints, port_directions, instances_by_name, modules);
        let net_name = mapped_net_name(&endpoints, indices, &raw_nets.net_names, ports);
        let mut net = Net::new(net_name.clone());
        net.driver = drivers.into_iter().next();
        net.sinks = sinks;
        if net.driver.is_none() && net.sinks.is_empty() {
            continue;
        }

        attach_mapped_cell_pins(cells, &net, &net_name);
        nets.push(net);
    }
    nets
}

fn mapped_net_endpoints(
    endpoints: &BTreeSet<MappedXmlEndpointRef>,
    port_directions: &BTreeMap<String, PortDirection>,
    instances_by_name: &BTreeMap<&str, &MappedXmlInstance>,
    modules: &BTreeMap<String, MappedXmlModule>,
) -> (Vec<Endpoint>, Vec<Endpoint>) {
    let mut drivers = Vec::<Endpoint>::new();
    let mut sinks = Vec::<Endpoint>::new();
    for endpoint in endpoints {
        match endpoint {
            MappedXmlEndpointRef::Port { name } => {
                let direction = port_directions
                    .get(name)
                    .cloned()
                    .unwrap_or(PortDirection::Input);
                let endpoint = Endpoint::port(name.clone(), name.clone());
                if direction.is_input_like() {
                    drivers.push(endpoint);
                } else if direction.is_output_like() {
                    sinks.push(endpoint);
                }
            }
            MappedXmlEndpointRef::Cell { instance, pin } => {
                let Some(mapped_instance) = instances_by_name.get(instance.as_str()).copied()
                else {
                    continue;
                };
                if !mapped_instance.keep {
                    continue;
                }
                let direction = modules
                    .get(&mapped_instance.module_ref)
                    .and_then(|module| module.port_directions.get(pin))
                    .cloned()
                    .unwrap_or(PortDirection::Input);
                let endpoint = Endpoint::cell(instance.clone(), pin.clone());
                if direction.is_output_like() {
                    drivers.push(endpoint);
                } else if direction.is_input_like() {
                    sinks.push(endpoint);
                }
            }
        }
    }
    (drivers, sinks)
}

fn attach_mapped_cell_pins(cells: &mut BTreeMap<String, Cell>, net: &Net, net_name: &str) {
    if let Some(driver) = &net.driver
        && let EndpointKind::Cell = driver.kind
        && let Some(cell) = cells.get_mut(&driver.name)
    {
        cell.outputs
            .push(CellPin::new(driver.pin.clone(), net_name.to_string()));
    }
    for sink in &net.sinks {
        if let EndpointKind::Cell = sink.kind
            && let Some(cell) = cells.get_mut(&sink.name)
        {
            cell.inputs
                .push(CellPin::new(sink.pin.clone(), net_name.to_string()));
        }
    }
}

fn mapped_ports(node: Node<'_, '_>) -> Vec<Port> {
    let direction = attr(&node, "direction")
        .parse()
        .unwrap_or(PortDirection::Input);
    expand_bus_port_names(node)
        .into_iter()
        .map(|name| {
            let mut port = Port::new(name, direction.clone());
            port.width = 1;
            port
        })
        .collect()
}

fn mapped_modules(root: Node<'_, '_>) -> BTreeMap<String, MappedXmlModule> {
    root.children()
        .filter(|node| node.has_tag_name("external"))
        .flat_map(|external| {
            external
                .children()
                .filter(|node| node.has_tag_name("module"))
        })
        .map(|module| {
            let port_directions = module
                .children()
                .filter(|node| node.has_tag_name("port"))
                .map(|port| {
                    (
                        attr(&port, "name"),
                        attr(&port, "direction")
                            .parse()
                            .unwrap_or(PortDirection::Input),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            (
                attr(&module, "name"),
                MappedXmlModule {
                    type_name: attr(&module, "type"),
                    port_directions,
                },
            )
        })
        .collect()
}

fn mapped_instance(
    node: &Node<'_, '_>,
    modules: &BTreeMap<String, MappedXmlModule>,
) -> Result<MappedXmlInstance> {
    let name = attr(node, "name");
    let module_ref = attr(node, "moduleRef");
    let module = modules
        .get(&module_ref)
        .ok_or_else(|| anyhow!("module definition for {module_ref} not found"))?;
    let cell = mapped_cell(name.clone(), &module_ref, module);
    let keep = matches!(cell.kind, CellKind::Lut | CellKind::Ff | CellKind::Constant);
    let mut instance = MappedXmlInstance {
        name,
        module_ref,
        keep,
        cell,
    };
    for property in node
        .children()
        .filter(|child| child.has_tag_name("property"))
    {
        let key = attr(&property, "name");
        let value =
            normalized_mapped_property_value(&instance.cell, &key, attr(&property, "value"));
        if instance.keep {
            let key = if key.eq_ignore_ascii_case("INIT") {
                "lut_init".to_string()
            } else {
                key
            };
            instance.cell.properties.push(Property::new(key, value));
        }
    }
    Ok(instance)
}

fn normalized_mapped_property_value(cell: &Cell, key: &str, value: String) -> String {
    if cell.is_lut() && key.eq_ignore_ascii_case("INIT") {
        let trimmed = value.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("0x")
            && !trimmed.starts_with("0X")
            && !trimmed.contains('\'')
        {
            return format!("0x{trimmed}");
        }
    }
    value
}

fn mapped_cell(name: String, module_ref: &str, module: &MappedXmlModule) -> Cell {
    let (kind, type_name) = if module_ref.eq_ignore_ascii_case("LOGIC_1") {
        (CellKind::Constant, "VCC".to_string())
    } else if module_ref.eq_ignore_ascii_case("LOGIC_0") {
        (CellKind::Constant, "GND".to_string())
    } else if module.type_name.eq_ignore_ascii_case("LUT") {
        (CellKind::Lut, module_ref.to_string())
    } else if module.type_name.eq_ignore_ascii_case("FFLATCH") {
        (CellKind::Ff, module_ref.to_string())
    } else if module_ref.contains("BUF") {
        (CellKind::Buffer, module_ref.to_string())
    } else {
        (CellKind::Unknown, module_ref.to_string())
    };
    Cell::new(name, kind, type_name)
}

fn is_physical_site_instance(module_ref: &str) -> bool {
    matches!(module_ref, "slice" | "iob" | "gclk" | "gclkiob")
}

fn mapped_net_name(
    endpoints: &BTreeSet<MappedXmlEndpointRef>,
    indices: &[usize],
    net_names: &[String],
    ports: &[Port],
) -> String {
    let port_names = ports
        .iter()
        .map(|port| port.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(port_name) = endpoints.iter().find_map(|endpoint| match endpoint {
        MappedXmlEndpointRef::Port { name } if port_names.contains(name.as_str()) => Some(name),
        MappedXmlEndpointRef::Port { .. } | MappedXmlEndpointRef::Cell { .. } => None,
    }) {
        return port_name.clone();
    }
    if let Some(port_name) = indices
        .iter()
        .map(|&index| net_names[index].as_str())
        .find(|name| port_names.contains(name))
    {
        return port_name.to_string();
    }
    indices
        .iter()
        .map(|&index| net_names[index].as_str())
        .find(|name| !name.starts_with("net_"))
        .unwrap_or_else(|| net_names[indices[0]].as_str())
        .to_string()
}

fn lower_mapped_constant_sources(design: &mut Design) {
    let lut_size = design.metadata.lut_size.max(1);
    let mut lowered = BTreeSet::new();

    for (cell_index, cell) in design.cells.iter_mut().enumerate() {
        let Some(init) = mapped_constant_lut_init(cell, lut_size) else {
            continue;
        };
        if cell.outputs.is_empty() {
            continue;
        }
        cell.kind = CellKind::Lut;
        cell.type_name = format!("LUT{lut_size}");
        cell.inputs.clear();
        for output in &mut cell.outputs {
            output.port = "O".to_string();
        }
        cell.set_property("lut_init", init);
        lowered.insert(cell_index);
    }

    if lowered.is_empty() {
        return;
    }

    let lowered_driver_nets = {
        let index = design.index();
        design
            .nets
            .iter()
            .map(|net| {
                net.driver
                    .as_ref()
                    .and_then(|driver| index.cell_for_endpoint(driver))
                    .is_some_and(|cell_id| lowered.contains(&cell_id.index()))
            })
            .collect::<Vec<_>>()
    };

    for (net, is_lowered_driver) in design.nets.iter_mut().zip(lowered_driver_nets) {
        if is_lowered_driver && let Some(driver) = &mut net.driver {
            driver.pin = "O".to_string();
        }
    }
}

fn mapped_constant_lut_init(cell: &Cell, lut_size: usize) -> Option<String> {
    match cell.constant_kind()? {
        crate::domain::ConstantKind::Zero => Some(format_lut_init_hex(0, lut_size)),
        crate::domain::ConstantKind::One => {
            let bits = 1usize.checked_shl(lut_size.min(7) as u32).unwrap_or(128);
            let value = if bits >= 128 {
                u128::MAX
            } else {
                (1u128 << bits) - 1
            };
            Some(format_lut_init_hex(value, lut_size))
        }
        crate::domain::ConstantKind::Unknown => None,
    }
}

fn format_lut_init_hex(value: u128, lut_width: usize) -> String {
    let bit_count = 1usize.checked_shl(lut_width.min(7) as u32).unwrap_or(128);
    let masked = if bit_count >= 128 {
        value
    } else {
        value & ((1u128 << bit_count) - 1)
    };
    let digits = match lut_width {
        0..=2 => 1,
        _ => 1usize << lut_width.saturating_sub(2).min(5),
    };
    format!("0x{masked:0digits$X}")
}

fn disjoint_find(parent: &mut [usize], index: usize) -> usize {
    if parent[index] != index {
        let root = disjoint_find(parent, parent[index]);
        parent[index] = root;
    }
    parent[index]
}

fn disjoint_union(parent: &mut [usize], lhs: usize, rhs: usize) {
    let lhs_root = disjoint_find(parent, lhs);
    let rhs_root = disjoint_find(parent, rhs);
    if lhs_root != rhs_root {
        parent[rhs_root] = lhs_root;
    }
}

#[cfg(test)]
mod tests;
