use super::helpers::attr;
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

pub(super) fn load_fde_mapped_design_xml(root: Node<'_, '_>) -> Result<Design> {
    let top_module_ref = root
        .children()
        .find(|node| node.has_tag_name("topModule"))
        .ok_or_else(|| anyhow!("missing <topModule> section"))?;
    let library_name = attr(&top_module_ref, "libraryRef");
    let module_name = attr(&top_module_ref, "name");

    let modules = mapped_modules(root);
    let module_node = root
        .children()
        .find(|node| node.has_tag_name("library") && attr(node, "name") == library_name)
        .and_then(|library| {
            library
                .children()
                .find(|node| node.has_tag_name("module") && attr(node, "name") == module_name)
        })
        .ok_or_else(|| anyhow!("FDE top module {module_name} not found"))?;
    let contents = module_node
        .children()
        .find(|node| node.has_tag_name("contents"))
        .ok_or_else(|| anyhow!("FDE design is missing <contents>"))?;

    let ports = module_node
        .children()
        .filter(|node| node.has_tag_name("port"))
        .map(|port| {
            Port::new(
                attr(&port, "name"),
                attr(&port, "direction")
                    .parse()
                    .unwrap_or(PortDirection::Input),
            )
        })
        .collect::<Vec<_>>();
    let port_directions = ports
        .iter()
        .map(|port| (port.name.clone(), port.direction.clone()))
        .collect::<BTreeMap<_, _>>();

    let instances = contents
        .children()
        .filter(|node| node.has_tag_name("instance"))
        .map(|node| mapped_instance(&node, &modules))
        .collect::<Result<Vec<_>>>()?;

    if instances
        .iter()
        .any(|instance| is_physical_site_instance(&instance.module_ref))
    {
        bail!("direct physical packed/placed/routed FDE XML import is not implemented");
    }

    let mut net_names = Vec::new();
    let mut raw_nets = Vec::<Vec<MappedXmlEndpointRef>>::new();
    let mut instance_nets = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for net in contents.children().filter(|node| node.has_tag_name("net")) {
        let net_index = raw_nets.len();
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
        raw_nets.push(endpoints);
    }

    let mut parent = (0..raw_nets.len()).collect::<Vec<_>>();
    for instance in &instances {
        if instance.keep {
            continue;
        }
        let Some(module) = modules.get(&instance.module_ref) else {
            continue;
        };
        let Some(pin_nets) = instance_nets.get(&instance.name) else {
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
    for net_index in 0..raw_nets.len() {
        let root_index = disjoint_find(&mut parent, net_index);
        grouped_indices
            .entry(root_index)
            .or_default()
            .push(net_index);
    }

    let mut cells = instances
        .iter()
        .filter(|instance| instance.keep)
        .map(|instance| (instance.name.clone(), instance.cell.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut nets = Vec::new();
    for indices in grouped_indices.values() {
        let mut endpoints = BTreeSet::<MappedXmlEndpointRef>::new();
        for &net_index in indices {
            endpoints.extend(raw_nets[net_index].iter().cloned());
        }

        let mut drivers = Vec::<Endpoint>::new();
        let mut sinks = Vec::<Endpoint>::new();
        for endpoint in &endpoints {
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
                    let Some(mapped_instance) = instances
                        .iter()
                        .find(|candidate| candidate.name == *instance)
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

        let net_name = mapped_net_name(&endpoints, indices, &net_names, &ports);
        let mut net = Net::new(net_name.clone());
        net.driver = drivers.into_iter().next();
        net.sinks = sinks;
        if net.driver.is_none() && net.sinks.is_empty() {
            continue;
        }

        if let Some(driver) = &net.driver
            && let EndpointKind::Cell = driver.kind
            && let Some(cell) = cells.get_mut(&driver.name)
        {
            cell.outputs
                .push(CellPin::new(driver.pin.clone(), net_name.clone()));
        }
        for sink in &net.sinks {
            if let EndpointKind::Cell = sink.kind
                && let Some(cell) = cells.get_mut(&sink.name)
            {
                cell.inputs
                    .push(CellPin::new(sink.pin.clone(), net_name.clone()));
            }
        }
        nets.push(net);
    }

    Ok(Design {
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
    })
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
            && !trimmed.starts_with("0b")
            && !trimmed.starts_with("0B")
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
mod tests {
    use super::load_fde_mapped_design_xml;
    use roxmltree::Document;

    #[test]
    fn mapped_lut_init_bare_values_are_imported_as_hex() {
        let xml = r#"
<design name="demo">
  <external name="cell_lib">
    <module name="LUT3" type="LUT">
      <port name="ADR0" direction="input"/>
      <port name="ADR1" direction="input"/>
      <port name="ADR2" direction="input"/>
      <port name="O" direction="output"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="demo" type="GENERIC">
      <contents>
        <instance name="id00001" moduleRef="LUT3" libraryRef="cell_lib">
          <property name="INIT" value="96"/>
        </instance>
      </contents>
    </module>
  </library>
  <topModule name="demo" libraryRef="work_lib"/>
</design>
"#;
        let doc = Document::parse(xml).expect("xml parse");
        let design = load_fde_mapped_design_xml(doc.root_element()).expect("mapped xml import");
        let cell = design
            .cells
            .iter()
            .find(|cell| cell.name == "id00001")
            .expect("lut cell");
        assert_eq!(cell.property("lut_init"), Some("0x96"));
    }
}
