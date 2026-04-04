use crate::{
    domain::PinRole,
    ir::{Design, DesignIndex, Endpoint},
};
use std::collections::BTreeMap;

use super::super::writer::{
    PhysicalEndpoint, PhysicalNet, PortInstanceBinding, SliceCellBinding, pin_map_indices,
};
use super::ports::split_clock_route_pips;

type PortLookup<'a> = BTreeMap<&'a str, &'a PortInstanceBinding>;

pub(super) fn build_physical_nets(
    design: &Design,
    index: &DesignIndex<'_>,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_bindings: &[PortInstanceBinding],
) -> Vec<PhysicalNet> {
    let port_lookup = port_lookup(port_bindings);
    let mut nets = build_internal_physical_nets(design, index, cell_bindings, &port_lookup);
    nets.extend(build_port_physical_nets(design, port_bindings));
    nets
}

fn port_lookup(port_bindings: &[PortInstanceBinding]) -> PortLookup<'_> {
    port_bindings
        .iter()
        .map(|binding| (binding.port_name.as_str(), binding))
        .collect()
}

fn build_internal_physical_nets(
    design: &Design,
    index: &DesignIndex<'_>,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_lookup: &PortLookup<'_>,
) -> Vec<PhysicalNet> {
    design
        .nets
        .iter()
        .filter_map(|net| {
            build_internal_physical_net(net, design, index, cell_bindings, port_lookup)
        })
        .collect()
}

fn build_internal_physical_net(
    net: &crate::ir::Net,
    design: &Design,
    index: &DesignIndex<'_>,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_lookup: &PortLookup<'_>,
) -> Option<PhysicalNet> {
    let driver_port_binding = net
        .driver
        .as_ref()
        .and_then(|driver| resolved_port_binding(driver, design, index, port_lookup));
    let sink_port_binding = net
        .sinks
        .iter()
        .find_map(|sink| resolved_port_binding(sink, design, index, port_lookup));
    let driver_binding = net
        .driver
        .as_ref()
        .and_then(|driver| resolved_cell_binding(driver, design, index, cell_bindings));
    let endpoints = internal_net_endpoints(net, design, cell_bindings, port_lookup, driver_binding);
    if endpoints.len() < 2 {
        return None;
    }

    Some(PhysicalNet {
        name: physical_internal_net_name(net, driver_port_binding, sink_port_binding),
        net_type: is_clock_buffer_binding(driver_port_binding).then_some("clock"),
        endpoints,
        pips: internal_net_pips(net, driver_port_binding),
    })
}

fn resolved_port_binding<'a>(
    endpoint: &Endpoint,
    design: &Design,
    index: &DesignIndex<'_>,
    port_lookup: &'a PortLookup<'a>,
) -> Option<&'a PortInstanceBinding> {
    match index.resolve_endpoint(endpoint) {
        crate::ir::EndpointTarget::Port(port_id) => {
            let port = index.port(design, port_id);
            port_lookup.get(port.name.as_str()).copied()
        }
        crate::ir::EndpointTarget::Cell(_) | crate::ir::EndpointTarget::Unknown => None,
    }
}

fn resolved_cell_binding(
    endpoint: &Endpoint,
    design: &Design,
    index: &DesignIndex<'_>,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
) -> Option<SliceCellBinding> {
    match index.resolve_endpoint(endpoint) {
        crate::ir::EndpointTarget::Cell(cell_id) => {
            let cell = index.cell(design, cell_id);
            cell_bindings
                .get(cell.name.as_str())
                .map(|(_, binding)| *binding)
        }
        crate::ir::EndpointTarget::Port(_) | crate::ir::EndpointTarget::Unknown => None,
    }
}

fn internal_net_endpoints(
    net: &crate::ir::Net,
    design: &Design,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_lookup: &PortLookup<'_>,
    driver_binding: Option<SliceCellBinding>,
) -> Vec<PhysicalEndpoint> {
    let mut endpoints = Vec::new();
    if let Some(driver) = &net.driver
        && let Some(endpoint) = physical_driver_endpoint(driver, design, cell_bindings, port_lookup)
    {
        push_unique_endpoint(&mut endpoints, endpoint);
    }
    for sink in &net.sinks {
        for endpoint in physical_sink_endpoints(
            sink,
            net.driver.as_ref(),
            design,
            cell_bindings,
            port_lookup,
            driver_binding,
        ) {
            push_unique_endpoint(&mut endpoints, endpoint);
        }
    }
    endpoints
}

fn is_clock_buffer_binding(binding: Option<&PortInstanceBinding>) -> bool {
    binding.is_some_and(|binding| binding.clock_input && binding.gclk_instance_name.is_some())
}

fn internal_net_pips(
    net: &crate::ir::Net,
    driver_port_binding: Option<&PortInstanceBinding>,
) -> Vec<crate::ir::RoutePip> {
    driver_port_binding
        .filter(|binding| is_clock_buffer_binding(Some(binding)))
        .map(|binding| split_clock_route_pips(&net.route_pips, binding).0)
        .unwrap_or_else(|| net.route_pips.clone())
}

fn build_port_physical_nets(
    design: &Design,
    port_bindings: &[PortInstanceBinding],
) -> Vec<PhysicalNet> {
    port_bindings
        .iter()
        .flat_map(|binding| port_physical_nets_for_binding(design, binding))
        .collect()
}

fn port_physical_nets_for_binding(
    design: &Design,
    binding: &PortInstanceBinding,
) -> Vec<PhysicalNet> {
    let mut nets = Vec::new();
    if binding.direction.is_input_like() {
        nets.push(input_port_pad_net(binding));
        if let Some(gclk_net) = input_clock_helper_net(design, binding) {
            nets.push(gclk_net);
        }
    }
    if binding.direction.is_output_like() {
        nets.push(output_port_pad_net(binding));
    }
    nets
}

fn input_port_pad_net(binding: &PortInstanceBinding) -> PhysicalNet {
    PhysicalNet {
        name: binding.port_name.clone(),
        net_type: None,
        endpoints: vec![
            PhysicalEndpoint {
                pin: binding.port_name.clone(),
                instance_ref: None,
            },
            PhysicalEndpoint {
                pin: "PAD".to_string(),
                instance_ref: Some(binding.pad_instance_name.clone()),
            },
        ],
        pips: Vec::new(),
    }
}

fn input_clock_helper_net(design: &Design, binding: &PortInstanceBinding) -> Option<PhysicalNet> {
    let gclk_instance_name = binding.gclk_instance_name.as_ref()?;
    Some(PhysicalNet {
        name: format!("net_Buf-pad-{}", binding.port_name),
        net_type: is_routed_physical_stage(design).then_some("clock"),
        endpoints: vec![
            PhysicalEndpoint {
                pin: "GCLKOUT".to_string(),
                instance_ref: Some(binding.pad_instance_name.clone()),
            },
            PhysicalEndpoint {
                pin: "IN".to_string(),
                instance_ref: Some(gclk_instance_name.clone()),
            },
        ],
        pips: helper_clock_pips(design, binding),
    })
}

fn helper_clock_pips(design: &Design, binding: &PortInstanceBinding) -> Vec<crate::ir::RoutePip> {
    if !is_routed_physical_stage(design) {
        return Vec::new();
    }
    design
        .nets
        .iter()
        .find(|net| net.name == binding.port_name)
        .map(|net| split_clock_route_pips(&net.route_pips, binding).1)
        .unwrap_or_default()
}

fn output_port_pad_net(binding: &PortInstanceBinding) -> PhysicalNet {
    PhysicalNet {
        name: binding.port_name.clone(),
        net_type: None,
        endpoints: vec![
            PhysicalEndpoint {
                pin: "PAD".to_string(),
                instance_ref: Some(binding.pad_instance_name.clone()),
            },
            PhysicalEndpoint {
                pin: binding.port_name.clone(),
                instance_ref: None,
            },
        ],
        pips: Vec::new(),
    }
}

fn is_routed_physical_stage(design: &Design) -> bool {
    matches!(design.stage.as_str(), "routed" | "timed")
}

fn physical_internal_net_name(
    net: &crate::ir::Net,
    driver_port_binding: Option<&PortInstanceBinding>,
    sink_port_binding: Option<&PortInstanceBinding>,
) -> String {
    if let Some(binding) = driver_port_binding {
        if binding.clock_input && binding.gclk_instance_name.is_some() {
            return format!("net_IBuf-clkpad-{}", binding.port_name);
        }
        return format!("net_Buf-pad-{}", binding.port_name);
    }
    if let Some(binding) = sink_port_binding {
        return format!("net_Buf-pad-{}", binding.port_name);
    }
    net.name.clone()
}

fn physical_driver_endpoint(
    endpoint: &Endpoint,
    design: &Design,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_lookup: &BTreeMap<&str, &PortInstanceBinding>,
) -> Option<PhysicalEndpoint> {
    match endpoint.kind {
        crate::domain::EndpointKind::Cell => {
            let cell = design
                .cells
                .iter()
                .find(|cell| cell.name == endpoint.name)?;
            let (instance_name, binding) = cell_bindings.get(cell.name.as_str())?;
            let pin = match PinRole::classify_output_pin(cell.primitive_kind(), &endpoint.pin) {
                PinRole::RegisterOutput => if binding.slot == 0 { "XQ" } else { "YQ" }.to_string(),
                PinRole::LutOutput => if binding.slot == 0 { "X" } else { "Y" }.to_string(),
                _ => return None,
            };
            Some(PhysicalEndpoint {
                pin,
                instance_ref: Some(instance_name.clone()),
            })
        }
        crate::domain::EndpointKind::Port => {
            let binding = port_lookup.get(endpoint.name.as_str())?;
            if let Some(gclk_instance_name) = binding.gclk_instance_name.as_ref() {
                Some(PhysicalEndpoint {
                    pin: "OUT".to_string(),
                    instance_ref: Some(gclk_instance_name.clone()),
                })
            } else {
                Some(PhysicalEndpoint {
                    pin: "IN".to_string(),
                    instance_ref: Some(binding.pad_instance_name.clone()),
                })
            }
        }
        crate::domain::EndpointKind::Unknown => None,
    }
}

fn physical_sink_endpoints(
    endpoint: &Endpoint,
    driver: Option<&Endpoint>,
    design: &Design,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    port_lookup: &BTreeMap<&str, &PortInstanceBinding>,
    driver_binding: Option<SliceCellBinding>,
) -> Vec<PhysicalEndpoint> {
    match endpoint.kind {
        crate::domain::EndpointKind::Cell => {
            let Some(cell) = design.cells.iter().find(|cell| cell.name == endpoint.name) else {
                return Vec::new();
            };
            let Some((instance_name, binding)) = cell_bindings.get(cell.name.as_str()) else {
                return Vec::new();
            };
            match PinRole::classify_for_primitive(cell.primitive_kind(), &endpoint.pin) {
                PinRole::LutInput(logical_index) => pin_map_indices(cell, logical_index)
                    .into_iter()
                    .map(|physical_index| PhysicalEndpoint {
                        pin: if binding.slot == 0 {
                            format!("F{}", physical_index + 1)
                        } else {
                            format!("G{}", physical_index + 1)
                        },
                        instance_ref: Some(instance_name.clone()),
                    })
                    .collect(),
                PinRole::RegisterClock => vec![PhysicalEndpoint {
                    pin: "CLK".to_string(),
                    instance_ref: Some(instance_name.clone()),
                }],
                PinRole::RegisterClockEnable => vec![PhysicalEndpoint {
                    pin: "CE".to_string(),
                    instance_ref: Some(instance_name.clone()),
                }],
                PinRole::RegisterSetReset => vec![PhysicalEndpoint {
                    pin: "SR".to_string(),
                    instance_ref: Some(instance_name.clone()),
                }],
                PinRole::RegisterData => {
                    if register_uses_local_lut(
                        driver,
                        design,
                        cell_bindings,
                        *binding,
                        driver_binding,
                    ) {
                        Vec::new()
                    } else {
                        vec![PhysicalEndpoint {
                            pin: if binding.slot == 0 { "BX" } else { "BY" }.to_string(),
                            instance_ref: Some(instance_name.clone()),
                        }]
                    }
                }
                _ => Vec::new(),
            }
        }
        crate::domain::EndpointKind::Port => port_lookup
            .get(endpoint.name.as_str())
            .map(|binding| PhysicalEndpoint {
                pin: "OUT".to_string(),
                instance_ref: Some(binding.pad_instance_name.clone()),
            })
            .into_iter()
            .collect(),
        crate::domain::EndpointKind::Unknown => Vec::new(),
    }
}

fn register_uses_local_lut(
    driver: Option<&Endpoint>,
    design: &Design,
    cell_bindings: &BTreeMap<String, (String, SliceCellBinding)>,
    sink_binding: SliceCellBinding,
    driver_binding: Option<SliceCellBinding>,
) -> bool {
    let Some(driver) = driver else {
        return false;
    };
    let crate::domain::EndpointKind::Cell = driver.kind else {
        return false;
    };
    let Some(driver_cell) = design.cells.iter().find(|cell| cell.name == driver.name) else {
        return false;
    };
    let Some((_, binding)) = cell_bindings.get(driver_cell.name.as_str()) else {
        return driver_binding.is_some_and(|binding| {
            driver_cell.is_lut() && binding.slot.min(1) == sink_binding.slot.min(1)
        });
    };
    driver_cell.is_lut() && binding.slot.min(1) == sink_binding.slot.min(1)
}

fn push_unique_endpoint(endpoints: &mut Vec<PhysicalEndpoint>, endpoint: PhysicalEndpoint) {
    if !endpoints.contains(&endpoint) {
        endpoints.push(endpoint);
    }
}
