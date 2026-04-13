use super::{ArraySpec, ParsedMember, ParsedName};
use crate::{
    domain::{CellKind, PinRole, PrimitiveKind},
    ir::{Cell, CellPin, Design, Endpoint, EndpointKind, Net, Port, PortDirection},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(super) struct PendingEndpoint {
    pub(super) pin: String,
    pub(super) target: EndpointTarget,
}

#[derive(Debug, Clone)]
pub(super) enum EndpointTarget {
    Port(String),
    InstanceRef(String),
}

#[derive(Debug, Clone)]
pub(super) struct PendingNet {
    pub(super) name: String,
    pub(super) endpoints: Vec<PendingEndpoint>,
}

#[derive(Debug, Clone)]
pub(super) struct DesignBuilder {
    pub(super) top_name: String,
    design: Design,
    cell_types: BTreeMap<String, String>,
    library_cell_names: BTreeMap<String, String>,
    library_cell_port_arrays: BTreeMap<String, BTreeMap<String, ArraySpec>>,
    instance_names: BTreeMap<String, String>,
    instance_types: BTreeMap<String, String>,
    pending_nets: Vec<PendingNet>,
}

impl DesignBuilder {
    pub(super) fn new(top_name: String) -> Self {
        let mut design = Design {
            name: top_name.clone(),
            stage: "mapped".to_string(),
            ..Design::default()
        };
        design.metadata.source_format = "edif".to_string();
        Self {
            top_name,
            design,
            cell_types: BTreeMap::new(),
            library_cell_names: BTreeMap::new(),
            library_cell_port_arrays: BTreeMap::new(),
            instance_names: BTreeMap::new(),
            instance_types: BTreeMap::new(),
            pending_nets: Vec::new(),
        }
    }

    pub(super) fn push_port(&mut self, port: Port) {
        self.design.ports.push(port);
    }

    pub(super) fn push_instance(&mut self, instance_ref: String, mut cell: Cell) {
        if let Some(resolved_type_name) = self.library_cell_names.get(&cell.type_name) {
            cell.type_name = resolved_type_name.clone();
        }
        cell.kind = classify_cell_kind(&cell.type_name);
        self.instance_names
            .insert(instance_ref.clone(), cell.name.clone());
        self.instance_types
            .insert(instance_ref, cell.type_name.clone());
        self.cell_types
            .insert(cell.name.clone(), cell.type_name.clone());
        self.design.cells.push(cell);
    }

    pub(super) fn register_library_cell(&mut self, name: ParsedName) {
        self.library_cell_names
            .insert(name.stable_name, name.display);
    }

    pub(super) fn register_library_cell_port_array(
        &mut self,
        cell_name: &str,
        array_key: &str,
        array_spec: ArraySpec,
    ) {
        self.library_cell_port_arrays
            .entry(cell_name.to_string())
            .or_default()
            .insert(array_key.to_string(), array_spec);
    }

    pub(super) fn resolve_instance_port_member(
        &self,
        instance_name: &str,
        member: &ParsedMember,
    ) -> Option<String> {
        let cell_type = self.instance_types.get(instance_name)?;
        self.library_cell_port_arrays
            .get(cell_type)?
            .get(&member.base_key)
            .and_then(|array_spec| {
                array_spec
                    .range
                    .member_name(&array_spec.display_base, member.ordinal)
            })
    }

    pub(super) fn push_net(&mut self, net: PendingNet) {
        self.pending_nets.push(net);
    }

    pub(super) fn finish(mut self) -> Design {
        for pending in self.pending_nets.drain(..) {
            let endpoints = pending
                .endpoints
                .into_iter()
                .map(|endpoint| match endpoint.target {
                    EndpointTarget::Port(name) => Endpoint {
                        kind: EndpointKind::Port,
                        name,
                        pin: endpoint.pin,
                    },
                    EndpointTarget::InstanceRef(instance_ref) => Endpoint {
                        kind: EndpointKind::Cell,
                        name: self
                            .instance_names
                            .get(&instance_ref)
                            .cloned()
                            .unwrap_or(instance_ref),
                        pin: endpoint.pin,
                    },
                })
                .collect::<Vec<_>>();
            let (driver, sinks) = split_endpoints(&self.design, &self.cell_types, &endpoints);
            self.design.nets.push(Net {
                name: pending.name,
                driver,
                sinks,
                ..Net::default()
            });
        }

        for cell in &mut self.design.cells {
            cell.inputs.clear();
            cell.outputs.clear();
        }
        let cell_index = self
            .design
            .cells
            .iter()
            .enumerate()
            .map(|(index, cell)| (cell.name.clone(), index))
            .collect::<BTreeMap<_, _>>();
        for net in &self.design.nets {
            if let Some(driver) = &net.driver
                && driver.is_cell()
                && let Some(index) = cell_index.get(&driver.name)
            {
                self.design.cells[*index]
                    .outputs
                    .push(CellPin::new(driver.pin.clone(), net.name.clone()));
            }
            for sink in &net.sinks {
                if sink.is_cell()
                    && let Some(index) = cell_index.get(&sink.name)
                {
                    self.design.cells[*index]
                        .inputs
                        .push(CellPin::new(sink.pin.clone(), net.name.clone()));
                }
            }
        }

        self.design
    }
}

fn split_endpoints(
    design: &Design,
    cell_types: &BTreeMap<String, String>,
    endpoints: &[Endpoint],
) -> (Option<Endpoint>, Vec<Endpoint>) {
    let port_dirs = design
        .ports
        .iter()
        .map(|port| (port.name.clone(), port.direction.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut sources = Vec::new();
    let mut sinks = Vec::new();
    for endpoint in endpoints {
        let is_source = match endpoint.kind {
            EndpointKind::Port => port_dirs
                .get(&endpoint.name)
                .map(PortDirection::is_input_like)
                .unwrap_or(false),
            EndpointKind::Cell => cell_types
                .get(&endpoint.name)
                .map(|type_name| is_output_pin(type_name, &endpoint.pin))
                .unwrap_or(false),
            EndpointKind::Unknown => false,
        };
        if is_source {
            sources.push(endpoint.clone());
        } else {
            sinks.push(endpoint.clone());
        }
    }

    let driver = sources
        .iter()
        .find(|endpoint| endpoint.is_cell())
        .cloned()
        .or_else(|| sources.first().cloned())
        .or_else(|| sinks.first().cloned());
    let sinks = endpoints
        .iter()
        .filter(|endpoint| Some(endpoint.key()) != driver.as_ref().map(Endpoint::key))
        .cloned()
        .collect::<Vec<_>>();
    (driver, sinks)
}

fn classify_cell_kind(type_name: &str) -> CellKind {
    match PrimitiveKind::classify("", type_name) {
        PrimitiveKind::Lut { .. } => CellKind::Lut,
        PrimitiveKind::FlipFlop => CellKind::Ff,
        PrimitiveKind::Latch => CellKind::Latch,
        PrimitiveKind::Constant(_) => CellKind::Constant,
        PrimitiveKind::Buffer => CellKind::Buffer,
        PrimitiveKind::Io => CellKind::Io,
        PrimitiveKind::GlobalClockBuffer => CellKind::GlobalClockBuffer,
        PrimitiveKind::BlockRam => CellKind::BlockRam,
        PrimitiveKind::Generic | PrimitiveKind::Unknown => CellKind::Generic,
    }
}

fn is_output_pin(type_name: &str, pin: &str) -> bool {
    PinRole::classify_output_pin(PrimitiveKind::classify("", type_name), pin).is_output_like()
}
