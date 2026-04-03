use super::{helpers::attr, lut_expr::decode_lut_function};
use crate::ir::{
    Cell, Cluster, Design, Endpoint, Net, Port, PortDirection, RoutePip, RouteSegment,
    SliceBindingKind,
};
use anyhow::{Result, anyhow};
use roxmltree::Node;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct PhysicalInstance {
    name: String,
    module_ref: String,
    position: Option<(usize, usize, usize)>,
    configs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
struct SliceSlotState {
    lut_name: Option<String>,
    ff_name: Option<String>,
    ff_clock_pin: String,
    ff_has_clock_enable: bool,
    ff_uses_local_lut: bool,
}

#[derive(Debug, Clone, Default)]
struct SliceState {
    instance_name: String,
    slots: [SliceSlotState; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalEndpointRole {
    Driver,
    Sink,
}

pub(super) fn load_fde_physical_design_xml(root: Node<'_, '_>) -> Result<Design> {
    let module_node = top_module_node(root)?;
    let contents = module_node
        .children()
        .find(|node| node.has_tag_name("contents"))
        .ok_or_else(|| anyhow!("FDE physical design is missing <contents>"))?;

    let mut ports = module_node
        .children()
        .filter(|node| node.has_tag_name("port"))
        .flat_map(physical_ports)
        .collect::<Vec<_>>();
    let port_names = ports
        .iter()
        .map(|port| port.name.clone())
        .collect::<BTreeSet<_>>();

    let instances = contents
        .children()
        .filter(|node| node.has_tag_name("instance"))
        .map(physical_instance)
        .collect::<Vec<_>>();
    let instances_by_name = instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();

    let clock_buffer_ports = clock_buffer_ports(&contents, &port_names);
    let clock_bridge_pips = clock_bridge_route_pips(&contents, &clock_buffer_ports);
    apply_port_positions(&mut ports, &instances_by_name);

    let mut cells = Vec::new();
    let mut clusters = Vec::new();
    let mut slice_states = BTreeMap::<String, SliceState>::new();
    for instance in instances
        .iter()
        .filter(|instance| instance.module_ref == "slice")
        .collect::<Vec<_>>()
    {
        let (cluster, mut cluster_cells, slice_state) = build_slice_cluster(instance);
        if !cluster.members.is_empty() {
            clusters.push(cluster);
        }
        cells.append(&mut cluster_cells);
        slice_states.insert(instance.name.clone(), slice_state);
    }

    clusters.sort_by(|lhs, rhs| {
        slice_instance_sort_key(&lhs.name).cmp(&slice_instance_sort_key(&rhs.name))
    });

    let mut nets = Vec::new();
    for net in contents.children().filter(|node| node.has_tag_name("net")) {
        let physical_name = attr(&net, "name");
        if is_pad_connection_net(&physical_name, &port_names) {
            continue;
        }
        if is_clock_bridge_net(&physical_name, &clock_buffer_ports) {
            continue;
        }

        let logical_name = logical_net_name(&physical_name, &port_names).to_string();
        let mut drivers = Vec::new();
        let mut sinks = Vec::new();
        for port_ref in net.children().filter(|node| node.has_tag_name("portRef")) {
            let Some(instance_name) = port_ref.attribute("instanceRef") else {
                continue;
            };
            let pin = attr(&port_ref, "name");
            for (endpoint, role) in physical_logical_endpoints(
                instance_name,
                &pin,
                &instances_by_name,
                &slice_states,
                &ports,
                &clock_buffer_ports,
            ) {
                match role {
                    PhysicalEndpointRole::Driver => push_unique_endpoint(&mut drivers, endpoint),
                    PhysicalEndpointRole::Sink => push_unique_endpoint(&mut sinks, endpoint),
                }
            }
        }

        if drivers.is_empty() && sinks.is_empty() {
            continue;
        }

        let route_pips = net
            .children()
            .filter(|node| node.has_tag_name("pip"))
            .filter_map(route_pip)
            .collect::<Vec<_>>();
        let route_pips = if let Some(helper_pips) = clock_bridge_pips.get(logical_name.as_str()) {
            merge_route_pips(helper_pips, route_pips)
        } else {
            route_pips
        };
        let route = derive_segments_from_pips(&route_pips);

        let mut imported = Net::new(logical_name);
        imported.driver = drivers.into_iter().next();
        imported.sinks = sinks;
        imported.route_pips = route_pips;
        imported.route = route;
        if imported.driver.is_none() && imported.sinks.is_empty() {
            continue;
        }
        nets.push(imported);
    }

    inject_local_lut_ff_nets(&slice_states, &mut nets);
    attach_cell_pins(&mut cells, &nets);

    let stage = infer_physical_stage(&instances, &nets);
    let note = match stage.as_str() {
        "packed" => "Imported FDE packed XML",
        "placed" => "Imported FDE placed XML",
        _ => "Imported FDE routed XML",
    };

    Ok(Design {
        name: attr(&root, "name"),
        stage,
        metadata: crate::ir::Metadata {
            source_format: "fde-xml".to_string(),
            notes: vec![note.to_string()],
            ..Default::default()
        },
        ports,
        cells,
        nets,
        clusters,
        ..Design::default()
    })
}

fn physical_ports(node: Node<'_, '_>) -> Vec<Port> {
    let direction = attr(&node, "direction")
        .parse()
        .unwrap_or(PortDirection::Input);
    let names = expanded_physical_port_names(node);
    names
        .into_iter()
        .map(|name| {
            let mut port = Port::new(name, direction.clone());
            port.width = 1;
            for property in node
                .children()
                .filter(|child| child.has_tag_name("property"))
            {
                let Some(value) = property.attribute("value") else {
                    continue;
                };
                match property.attribute("name") {
                    Some("fde_pin") => port.pin = Some(value.to_string()),
                    Some("fde_position") => {
                        if let Some((x, y, _)) = parse_point(value) {
                            port.x = Some(x);
                            port.y = Some(y);
                        }
                    }
                    _ => {}
                }
            }
            port
        })
        .collect()
}

fn expanded_physical_port_names(node: Node<'_, '_>) -> Vec<String> {
    let name = attr(&node, "name");
    let Some(msb) = node
        .attribute("msb")
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return vec![name];
    };
    let lsb = node
        .attribute("lsb")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(msb);
    if msb >= lsb {
        (lsb..=msb)
            .rev()
            .map(|index| format!("{name}[{index}]"))
            .collect()
    } else {
        (msb..=lsb)
            .map(|index| format!("{name}[{index}]"))
            .collect()
    }
}

fn top_module_node<'a, 'input>(root: Node<'a, 'input>) -> Result<Node<'a, 'input>> {
    let top_module_ref = root
        .children()
        .find(|node| node.has_tag_name("topModule"))
        .ok_or_else(|| anyhow!("missing <topModule> section"))?;
    let library_name = attr(&top_module_ref, "libraryRef");
    let module_name = attr(&top_module_ref, "name");
    root.children()
        .find(|node| node.has_tag_name("library") && attr(node, "name") == library_name)
        .and_then(|library| {
            library
                .children()
                .find(|node| node.has_tag_name("module") && attr(node, "name") == module_name)
        })
        .ok_or_else(|| anyhow!("FDE top module {module_name} not found"))
}

fn physical_instance(node: Node<'_, '_>) -> PhysicalInstance {
    let configs = node
        .children()
        .filter(|child| child.has_tag_name("config"))
        .map(|config| (attr(&config, "name"), attr(&config, "value")))
        .collect::<BTreeMap<_, _>>();
    PhysicalInstance {
        name: attr(&node, "name"),
        module_ref: attr(&node, "moduleRef"),
        position: instance_position(node),
        configs,
    }
}

fn clock_buffer_ports(
    contents: &Node<'_, '_>,
    port_names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let instance_modules = contents
        .children()
        .filter(|node| node.has_tag_name("instance"))
        .map(|instance| (attr(&instance, "name"), attr(&instance, "moduleRef")))
        .collect::<BTreeMap<_, _>>();

    let mut ports = BTreeMap::new();
    for net in contents.children().filter(|node| node.has_tag_name("net")) {
        let physical_name = attr(&net, "name");
        let Some(port_name) = physical_name.strip_prefix("net_Buf-pad-") else {
            continue;
        };
        if !port_names.contains(port_name) {
            continue;
        }
        let Some(gclk_instance_name) = net
            .children()
            .filter(|node| node.has_tag_name("portRef"))
            .filter(|port_ref| port_ref.attribute("name") == Some("IN"))
            .filter_map(|port_ref| port_ref.attribute("instanceRef"))
            .find(|instance_name| {
                instance_modules
                    .get(*instance_name)
                    .is_some_and(|module_ref| module_ref == "gclk")
            })
        else {
            continue;
        };
        ports.insert(gclk_instance_name.to_string(), port_name.to_string());
    }
    ports
}

fn clock_bridge_route_pips(
    contents: &Node<'_, '_>,
    clock_buffer_ports: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<RoutePip>> {
    let clock_port_names = clock_buffer_ports
        .values()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut pips_by_port = BTreeMap::<String, Vec<RoutePip>>::new();
    for net in contents.children().filter(|node| node.has_tag_name("net")) {
        let physical_name = attr(&net, "name");
        let Some(port_name) = physical_name.strip_prefix("net_Buf-pad-") else {
            continue;
        };
        if !clock_port_names.contains(port_name) {
            continue;
        }
        let helper_pips = net
            .children()
            .filter(|node| node.has_tag_name("pip"))
            .filter_map(route_pip)
            .collect::<Vec<_>>();
        if helper_pips.is_empty() {
            continue;
        }
        pips_by_port
            .entry(port_name.to_string())
            .or_default()
            .extend(helper_pips);
    }
    pips_by_port
}

fn apply_port_positions(ports: &mut [Port], instances_by_name: &BTreeMap<&str, &PhysicalInstance>) {
    for port in ports {
        let Some(instance) = instances_by_name.get(port.name.as_str()).copied() else {
            continue;
        };
        if !matches!(instance.module_ref.as_str(), "iob" | "gclkiob") {
            continue;
        }
        let Some((x, y, z)) = instance.position else {
            continue;
        };
        port.x = Some(x);
        port.y = Some(y);
        port.z = Some(z);
    }
}

fn build_slice_cluster(instance: &PhysicalInstance) -> (Cluster, Vec<Cell>, SliceState) {
    let mut cells = Vec::new();
    let mut members = Vec::new();
    let mut state = SliceState {
        instance_name: instance.name.clone(),
        ..Default::default()
    };

    for slot in 0..2 {
        let cfg_name = if slot == 0 { "F" } else { "G" };
        if let Some((lut_init, input_count)) = instance
            .configs
            .get(cfg_name)
            .and_then(|value| decode_lut_function(value))
        {
            let lut_name = format!("{}::lut{slot}", instance.name);
            let mut lut = Cell::lut(&lut_name, format!("LUT{input_count}"))
                .in_cluster(&instance.name)
                .with_slice_binding(slot, SliceBindingKind::Lut);
            lut.set_property("lut_init", lut_init);
            members.push(lut_name.clone());
            state.slots[slot].lut_name = Some(lut_name);
            cells.push(lut);
        }

        let ff_cfg_name = if slot == 0 { "FFX" } else { "FFY" };
        if instance
            .configs
            .get(ff_cfg_name)
            .is_none_or(|value| value == "#OFF")
        {
            continue;
        }
        let ff_name = format!("{}::ff{slot}", instance.name);
        let ff = Cell::ff(&ff_name, "DFFHQ")
            .in_cluster(&instance.name)
            .with_slice_binding(slot, SliceBindingKind::Sequential);
        members.push(ff_name.clone());
        state.slots[slot].ff_name = Some(ff_name);
        state.slots[slot].ff_clock_pin = if instance
            .configs
            .get("CKINV")
            .is_some_and(|value| value == "1")
        {
            "CKN".to_string()
        } else {
            "CK".to_string()
        };
        state.slots[slot].ff_has_clock_enable = instance
            .configs
            .get("CEMUX")
            .is_some_and(|value| value.eq_ignore_ascii_case("CE"));
        let d_cfg_name = if slot == 0 { "DXMUX" } else { "DYMUX" };
        state.slots[slot].ff_uses_local_lut = instance
            .configs
            .get(d_cfg_name)
            .is_none_or(|value| value == "1");
        cells.push(ff);
    }

    let mut cluster = Cluster::logic(&instance.name)
        .with_members(members)
        .with_capacity(4);
    if let Some((x, y, z)) = instance.position {
        cluster = cluster.fixed_at_slot(x, y, z);
    }
    (cluster, cells, state)
}

fn physical_logical_endpoints(
    instance_name: &str,
    pin: &str,
    instances_by_name: &BTreeMap<&str, &PhysicalInstance>,
    slice_states: &BTreeMap<String, SliceState>,
    ports: &[Port],
    clock_buffer_ports: &BTreeMap<String, String>,
) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    let Some(instance) = instances_by_name.get(instance_name).copied() else {
        return Vec::new();
    };
    match instance.module_ref.as_str() {
        "slice" => slice_logical_endpoints(instance_name, pin, slice_states),
        "iob" => port_logical_endpoints(instance_name, pin, ports),
        "gclk" => clock_buffer_ports
            .get(instance_name)
            .filter(|_| pin.eq_ignore_ascii_case("OUT"))
            .map(|port_name| {
                vec![(
                    Endpoint::port(port_name.clone(), port_name.clone()),
                    PhysicalEndpointRole::Driver,
                )]
            })
            .unwrap_or_default(),
        "gclkiob" => Vec::new(),
        _ => Vec::new(),
    }
}

fn slice_logical_endpoints(
    instance_name: &str,
    pin: &str,
    slice_states: &BTreeMap<String, SliceState>,
) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    let Some(state) = slice_states.get(instance_name) else {
        return Vec::new();
    };
    match pin {
        "X" => state.slots[0]
            .lut_name
            .as_ref()
            .map(|name| {
                vec![(
                    Endpoint::cell(name.clone(), "O"),
                    PhysicalEndpointRole::Driver,
                )]
            })
            .unwrap_or_default(),
        "Y" => state.slots[1]
            .lut_name
            .as_ref()
            .map(|name| {
                vec![(
                    Endpoint::cell(name.clone(), "O"),
                    PhysicalEndpointRole::Driver,
                )]
            })
            .unwrap_or_default(),
        "XQ" => state.slots[0]
            .ff_name
            .as_ref()
            .map(|name| {
                vec![(
                    Endpoint::cell(name.clone(), "Q"),
                    PhysicalEndpointRole::Driver,
                )]
            })
            .unwrap_or_default(),
        "YQ" => state.slots[1]
            .ff_name
            .as_ref()
            .map(|name| {
                vec![(
                    Endpoint::cell(name.clone(), "Q"),
                    PhysicalEndpointRole::Driver,
                )]
            })
            .unwrap_or_default(),
        "CLK" => ff_control_endpoints(&state.slots, |slot| {
            slot.ff_name
                .as_ref()
                .map(|name| Endpoint::cell(name.clone(), slot.ff_clock_pin.clone()))
        }),
        "CE" => ff_control_endpoints(&state.slots, |slot| {
            (slot.ff_has_clock_enable)
                .then(|| {
                    slot.ff_name
                        .as_ref()
                        .map(|name| Endpoint::cell(name.clone(), "E"))
                })
                .flatten()
        }),
        "SR" => ff_control_endpoints(&state.slots, |slot| {
            slot.ff_name
                .as_ref()
                .map(|name| Endpoint::cell(name.clone(), "RN"))
        }),
        "BX" => ff_bypass_endpoint(&state.slots[0]),
        "BY" => ff_bypass_endpoint(&state.slots[1]),
        _ => lut_input_endpoint(&state.slots, pin),
    }
}

fn ff_control_endpoints(
    slots: &[SliceSlotState; 2],
    mut endpoint: impl FnMut(&SliceSlotState) -> Option<Endpoint>,
) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    slots
        .iter()
        .filter_map(|slot| endpoint(slot).map(|endpoint| (endpoint, PhysicalEndpointRole::Sink)))
        .collect()
}

fn ff_bypass_endpoint(slot: &SliceSlotState) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    if slot.ff_uses_local_lut {
        return Vec::new();
    }
    slot.ff_name
        .as_ref()
        .map(|name| {
            vec![(
                Endpoint::cell(name.clone(), "D"),
                PhysicalEndpointRole::Sink,
            )]
        })
        .unwrap_or_default()
}

fn lut_input_endpoint(
    slots: &[SliceSlotState; 2],
    pin: &str,
) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    let (slot, input_index) = match pin {
        "F1" => (0, 0),
        "F2" => (0, 1),
        "F3" => (0, 2),
        "F4" => (0, 3),
        "G1" => (1, 0),
        "G2" => (1, 1),
        "G3" => (1, 2),
        "G4" => (1, 3),
        _ => return Vec::new(),
    };
    slots[slot]
        .lut_name
        .as_ref()
        .map(|name| {
            vec![(
                Endpoint::cell(name.clone(), format!("ADR{input_index}")),
                PhysicalEndpointRole::Sink,
            )]
        })
        .unwrap_or_default()
}

fn port_logical_endpoints(
    instance_name: &str,
    pin: &str,
    ports: &[Port],
) -> Vec<(Endpoint, PhysicalEndpointRole)> {
    let Some(port) = ports.iter().find(|port| port.name == instance_name) else {
        return Vec::new();
    };
    if port.direction.is_input_like() && pin.eq_ignore_ascii_case("IN") {
        return vec![(
            Endpoint::port(port.name.clone(), port.name.clone()),
            PhysicalEndpointRole::Driver,
        )];
    }
    if port.direction.is_output_like() && pin.eq_ignore_ascii_case("OUT") {
        return vec![(
            Endpoint::port(port.name.clone(), port.name.clone()),
            PhysicalEndpointRole::Sink,
        )];
    }
    Vec::new()
}

fn is_pad_connection_net(name: &str, port_names: &BTreeSet<String>) -> bool {
    port_names.contains(name)
}

fn is_clock_bridge_net(name: &str, clock_buffer_ports: &BTreeMap<String, String>) -> bool {
    name.strip_prefix("net_Buf-pad-").is_some_and(|port_name| {
        clock_buffer_ports
            .values()
            .any(|candidate| candidate == port_name)
    })
}

fn logical_net_name<'a>(physical_name: &'a str, port_names: &BTreeSet<String>) -> &'a str {
    physical_name
        .strip_prefix("net_IBuf-clkpad-")
        .filter(|name| port_names.contains(*name))
        .or_else(|| {
            physical_name
                .strip_prefix("net_Buf-pad-")
                .filter(|name| port_names.contains(*name))
        })
        .unwrap_or(physical_name)
}

fn inject_local_lut_ff_nets(slice_states: &BTreeMap<String, SliceState>, nets: &mut Vec<Net>) {
    for state in slice_states.values() {
        for slot in 0..2 {
            let Some(lut_name) = state.slots[slot].lut_name.as_ref() else {
                continue;
            };
            let Some(ff_name) = state.slots[slot].ff_name.as_ref() else {
                continue;
            };
            if !state.slots[slot].ff_uses_local_lut {
                continue;
            }
            let sink = Endpoint::cell(ff_name.clone(), "D");
            if let Some(existing) = nets.iter_mut().find(|net| {
                net.driver.as_ref().is_some_and(|driver| {
                    driver.kind == crate::domain::EndpointKind::Cell
                        && driver.name == *lut_name
                        && driver.pin.eq_ignore_ascii_case("O")
                })
            }) {
                push_unique_endpoint(&mut existing.sinks, sink);
                continue;
            }
            nets.push(
                Net::new(format!("{}::lut{slot}_to_ff{slot}", state.instance_name))
                    .with_driver(Endpoint::cell(lut_name.clone(), "O"))
                    .with_sink(sink),
            );
        }
    }
}

fn attach_cell_pins(cells: &mut [Cell], nets: &[Net]) {
    let cells_by_name = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| (cell.name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    for net in nets {
        if let Some(driver) = &net.driver
            && driver.kind == crate::domain::EndpointKind::Cell
            && let Some(&cell_index) = cells_by_name.get(&driver.name)
        {
            let cell = &mut cells[cell_index];
            if !cell
                .outputs
                .iter()
                .any(|pin| pin.port == driver.pin && pin.net == net.name)
            {
                cell.outputs.push(crate::ir::CellPin::new(
                    driver.pin.clone(),
                    net.name.clone(),
                ));
            }
        }
        for sink in &net.sinks {
            if sink.kind != crate::domain::EndpointKind::Cell {
                continue;
            }
            let Some(&cell_index) = cells_by_name.get(&sink.name) else {
                continue;
            };
            let cell = &mut cells[cell_index];
            if !cell
                .inputs
                .iter()
                .any(|pin| pin.port == sink.pin && pin.net == net.name)
            {
                cell.inputs
                    .push(crate::ir::CellPin::new(sink.pin.clone(), net.name.clone()));
            }
        }
    }
}

fn infer_physical_stage(instances: &[PhysicalInstance], nets: &[Net]) -> String {
    if nets.iter().any(|net| !net.route_pips.is_empty()) {
        return "routed".to_string();
    }
    if instances.iter().any(|instance| instance.position.is_some()) {
        return "placed".to_string();
    }
    "packed".to_string()
}

fn route_pip(pip: Node<'_, '_>) -> Option<RoutePip> {
    let (x, y) = pip_position(pip)?;
    Some(RoutePip::new(
        (x, y),
        pip.attribute("from")?.to_string(),
        pip.attribute("to")?.to_string(),
    ))
}

fn merge_route_pips(helper_pips: &[RoutePip], route_pips: Vec<RoutePip>) -> Vec<RoutePip> {
    let mut merged = helper_pips.to_vec();
    for pip in route_pips {
        if !merged.contains(&pip) {
            merged.push(pip);
        }
    }
    merged
}

fn derive_segments_from_pips(pips: &[RoutePip]) -> Vec<RouteSegment> {
    let mut positions = Vec::<(usize, usize)>::new();
    for pip in pips {
        let position = (pip.x, pip.y);
        if positions.last().copied() != Some(position) {
            positions.push(position);
        }
    }
    match positions.as_slice() {
        [] => Vec::new(),
        [single] => vec![RouteSegment::new(*single, *single)],
        _ => positions
            .windows(2)
            .filter_map(|window| match window {
                [start, end] => Some(RouteSegment::new(*start, *end)),
                _ => None,
            })
            .collect(),
    }
}

fn slice_instance_sort_key(name: &str) -> (usize, &str) {
    let index = name
        .strip_prefix("iSlice__")
        .and_then(|value| value.strip_suffix("__"))
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX);
    (index, name)
}

fn instance_position(instance: Node<'_, '_>) -> Option<(usize, usize, usize)> {
    instance
        .children()
        .find(|node| node.has_tag_name("property") && node.attribute("name") == Some("position"))
        .and_then(|property| property.attribute("value"))
        .and_then(parse_point)
}

fn pip_position(pip: Node<'_, '_>) -> Option<(usize, usize)> {
    let value = pip.attribute("position")?;
    let mut parts = value.split(',').map(str::trim);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    Some((x, y))
}

fn parse_point(value: &str) -> Option<(usize, usize, usize)> {
    let mut parts = value.split(',').map(str::trim);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let z = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((x, y, z))
}

fn push_unique_endpoint(endpoints: &mut Vec<Endpoint>, endpoint: Endpoint) {
    if endpoints
        .iter()
        .any(|existing| existing.key() == endpoint.key())
    {
        return;
    }
    endpoints.push(endpoint);
}

#[cfg(test)]
mod tests {
    use super::load_fde_physical_design_xml;
    use crate::ir::{RoutePip, RouteSegment};

    #[test]
    fn physical_import_merges_clock_bridge_pips_back_into_clock_net() {
        let xml = r##"
<design name="clock_import">
  <external name="template_work_lib">
    <module name="slice" type="SLICE">
      <port name="CLK" direction="input" capacitance="0.00000"/>
    </module>
    <module name="gclk" type="GCLK">
      <port name="IN" direction="input" capacitance="0.00000"/>
      <port name="OUT" direction="output" capacitance="0.00000"/>
    </module>
    <module name="gclkiob" type="GCLKIOB">
      <port name="GCLKOUT" direction="output" capacitance="0.00000"/>
      <port name="PAD" direction="inout" capacitance="0.00000"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="clock_import" type="GENERIC">
      <port name="clk" direction="input" capacitance="0.00000"/>
      <contents>
        <instance name="iSlice__0__" moduleRef="slice" libraryRef="template_work_lib">
          <property name="position" type="point" value="18,36,0"/>
          <config name="CKINV" value="1"/>
          <config name="DXMUX" value="1"/>
          <config name="FFX" value="#FF"/>
        </instance>
        <instance name="iGclk_buf__0__" moduleRef="gclk" libraryRef="template_work_lib">
          <property name="position" type="point" value="34,27,1"/>
        </instance>
        <instance name="clk" moduleRef="gclkiob" libraryRef="template_work_lib">
          <property name="position" type="point" value="34,27,1"/>
        </instance>
        <net name="net_IBuf-clkpad-clk" type="clock">
          <portRef name="OUT" instanceRef="iGclk_buf__0__"/>
          <portRef name="CLK" instanceRef="iSlice__0__"/>
          <pip from="CLKB_GCLK1_PW" to="CLKB_GCLK1" position="34,27" dir="-&gt;"/>
        </net>
        <net name="net_Buf-pad-clk" type="clock">
          <portRef name="GCLKOUT" instanceRef="clk"/>
          <portRef name="IN" instanceRef="iGclk_buf__0__"/>
          <pip from="CLKB_CLKPAD1" to="CLKB_GCLKBUF1_IN" position="34,27" dir="-&gt;"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule libraryRef="work_lib" name="clock_import"/>
</design>
"##;

        let document = roxmltree::Document::parse(xml).expect("physical XML should parse");
        let design = load_fde_physical_design_xml(document.root_element())
            .expect("physical import should succeed");
        let clock_net = design
            .nets
            .iter()
            .find(|net| net.name == "clk")
            .expect("clock net should be imported");

        assert_eq!(
            clock_net.route_pips,
            vec![
                RoutePip::new((34, 27), "CLKB_CLKPAD1", "CLKB_GCLKBUF1_IN"),
                RoutePip::new((34, 27), "CLKB_GCLK1_PW", "CLKB_GCLK1"),
            ]
        );
        assert_eq!(clock_net.route, vec![RouteSegment::new((34, 27), (34, 27))]);
    }

    #[test]
    fn physical_import_preserves_port_pin_and_site_slot() {
        let xml = r##"
<design name="port_import">
  <external name="template_work_lib">
    <module name="iob" type="IOB">
      <port name="OUT" direction="input" capacitance="0.00000"/>
      <port name="PAD" direction="inout" capacitance="0.00000"/>
    </module>
    <module name="gclk" type="GCLK">
      <port name="IN" direction="input" capacitance="0.00000"/>
      <port name="OUT" direction="output" capacitance="0.00000"/>
    </module>
    <module name="gclkiob" type="GCLKIOB">
      <port name="GCLKOUT" direction="output" capacitance="0.00000"/>
      <port name="PAD" direction="inout" capacitance="0.00000"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="port_import" type="GENERIC">
      <port name="led" direction="output" capacitance="0.00000">
        <property name="fde_pin" value="P7"/>
        <property name="fde_position" type="point" value="5,1"/>
      </port>
      <port name="clk" direction="input" capacitance="0.00000">
        <property name="fde_pin" value="P77"/>
        <property name="fde_position" type="point" value="34,27"/>
      </port>
      <contents>
        <instance name="led" moduleRef="iob" libraryRef="template_work_lib">
          <property name="position" type="point" value="5,1,2"/>
        </instance>
        <instance name="iGclk_buf__0__" moduleRef="gclk" libraryRef="template_work_lib">
          <property name="position" type="point" value="34,27,1"/>
        </instance>
        <instance name="clk" moduleRef="gclkiob" libraryRef="template_work_lib">
          <property name="position" type="point" value="34,27,1"/>
        </instance>
        <net name="net_Buf-pad-led">
          <portRef name="OUT" instanceRef="led"/>
          <portRef name="led"/>
        </net>
        <net name="led">
          <portRef name="PAD" instanceRef="led"/>
        </net>
        <net name="net_Buf-pad-clk" type="clock">
          <portRef name="GCLKOUT" instanceRef="clk"/>
          <portRef name="IN" instanceRef="iGclk_buf__0__"/>
          <pip from="CLKB_CLKPAD1" to="CLKB_GCLKBUF1_IN" position="34,27" dir="-&gt;"/>
        </net>
        <net name="clk" type="clock">
          <portRef name="OUT" instanceRef="iGclk_buf__0__"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule libraryRef="work_lib" name="port_import"/>
</design>
"##;

        let document = roxmltree::Document::parse(xml).expect("physical XML should parse");
        let design = load_fde_physical_design_xml(document.root_element())
            .expect("physical import should succeed");

        let led = design
            .ports
            .iter()
            .find(|port| port.name == "led")
            .expect("led port");
        assert_eq!(led.pin.as_deref(), Some("P7"));
        assert_eq!((led.x, led.y, led.z), (Some(5), Some(1), Some(2)));

        let clk = design
            .ports
            .iter()
            .find(|port| port.name == "clk")
            .expect("clk port");
        assert_eq!(clk.pin.as_deref(), Some("P77"));
        assert_eq!((clk.x, clk.y, clk.z), (Some(34), Some(27), Some(1)));
    }

    #[test]
    fn physical_import_expands_cpp_bus_ports_into_bit_ports() {
        let xml = r##"
<design name="bus_import">
  <external name="template_work_lib">
    <module name="iob" type="IOB">
      <port name="OUT" direction="input" capacitance="0.00000"/>
      <port name="PAD" direction="inout" capacitance="0.00000"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="bus_import" type="GENERIC">
      <port name="led" msb="1" lsb="0" direction="output" capacitance="0.00000"/>
      <contents>
        <instance name="led[1]" moduleRef="iob" libraryRef="template_work_lib">
          <property name="position" type="point" value="5,1,3"/>
        </instance>
        <instance name="led[0]" moduleRef="iob" libraryRef="template_work_lib">
          <property name="position" type="point" value="5,1,2"/>
        </instance>
        <net name="net_Buf-pad-led[1]">
          <portRef name="OUT" instanceRef="led[1]"/>
          <portRef name="led[1]"/>
        </net>
        <net name="led[1]">
          <portRef name="PAD" instanceRef="led[1]"/>
          <portRef name="led[1]"/>
        </net>
        <net name="net_Buf-pad-led[0]">
          <portRef name="OUT" instanceRef="led[0]"/>
          <portRef name="led[0]"/>
        </net>
        <net name="led[0]">
          <portRef name="PAD" instanceRef="led[0]"/>
          <portRef name="led[0]"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule libraryRef="work_lib" name="bus_import"/>
</design>
"##;

        let document = roxmltree::Document::parse(xml).expect("physical XML should parse");
        let design = load_fde_physical_design_xml(document.root_element())
            .expect("physical import should succeed");

        let port_names = design
            .ports
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(port_names, vec!["led[1]", "led[0]"]);

        let led1 = design
            .ports
            .iter()
            .find(|port| port.name == "led[1]")
            .expect("led[1] port");
        assert_eq!((led1.x, led1.y, led1.z), (Some(5), Some(1), Some(3)));

        let led0 = design
            .ports
            .iter()
            .find(|port| port.name == "led[0]")
            .expect("led[0] port");
        assert_eq!((led0.x, led0.y, led0.z), (Some(5), Some(1), Some(2)));

        let net_names = design
            .nets
            .iter()
            .map(|net| net.name.as_str())
            .collect::<Vec<_>>();
        assert!(net_names.contains(&"led[1]"));
        assert!(net_names.contains(&"led[0]"));
    }

    #[test]
    fn physical_import_preserves_cpp_constant_zero_lut_outputs() {
        let xml = r##"
<design name="const_zero_lut">
  <external name="template_work_lib">
    <module name="slice" type="SLICE">
      <port name="Y" direction="output" capacitance="0.00000"/>
    </module>
    <module name="iob" type="IOB">
      <port name="OUT" direction="input" capacitance="0.00000"/>
      <port name="PAD" direction="inout" capacitance="0.00000"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="const_zero_lut" type="GENERIC">
      <port name="led" direction="output" capacitance="0.00000"/>
      <contents>
        <instance name="iSlice__0__" moduleRef="slice" libraryRef="template_work_lib">
          <property name="position" type="point" value="27,23,0"/>
          <config name="F" value="#OFF"/>
          <config name="G" value="#LUT:D=0"/>
          <config name="FXMUX" value="#OFF"/>
          <config name="GYMUX" value="G"/>
          <config name="XUSED" value="#OFF"/>
          <config name="YUSED" value="0"/>
        </instance>
        <instance name="led" moduleRef="iob" libraryRef="template_work_lib">
          <property name="position" type="point" value="34,30,1"/>
        </instance>
        <net name="net_Buf-pad-led">
          <portRef name="Y" instanceRef="iSlice__0__"/>
          <portRef name="OUT" instanceRef="led"/>
          <pip from="S0_Y" to="OUT3" position="27,23" dir="-&gt;"/>
        </net>
        <net name="led">
          <portRef name="PAD" instanceRef="led"/>
          <portRef name="led"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule libraryRef="work_lib" name="const_zero_lut"/>
</design>
"##;

        let document = roxmltree::Document::parse(xml).expect("physical XML should parse");
        let design = load_fde_physical_design_xml(document.root_element())
            .expect("physical import should succeed");

        assert!(
            design
                .cells
                .iter()
                .any(|cell| cell.name == "iSlice__0__::lut1" && cell.property("lut_init") == Some("0x0"))
        );
        let led_net = design
            .nets
            .iter()
            .find(|net| net.name == "led")
            .expect("logical led net");
        assert_eq!(
            led_net.driver.as_ref().map(|endpoint| endpoint.name.as_str()),
            Some("iSlice__0__::lut1")
        );
    }
}
