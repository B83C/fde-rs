use crate::{
    bitgen::literal::{parse_bit_literal, parse_compact_hex_digit_literal},
    domain::{CellKind, PrimitiveKind, pin_map_property_name},
    ir::{Cell, Design, Endpoint, Net, Port, PortDirection, RoutePip, RouteSegment},
};
use anyhow::Result;
use quick_xml::events::{BytesDecl, BytesStart, Event};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    super::lut_expr::encode_lut_expression_literal, DesignXmlWriter, LOGICAL_EXTERNAL_LIB, WORK_LIB,
};

impl DesignXmlWriter {
    pub(super) fn write_logical_design(
        &mut self,
        emitted_design: &Design,
        external_lib: &str,
    ) -> Result<()> {
        self.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

        let mut root = BytesStart::new("design");
        root.push_attribute(("name", emitted_design.name.as_str()));
        self.start_element(root)?;

        self.write_external_library(emitted_design, false, external_lib)?;
        self.write_work_library(emitted_design, false, external_lib)?;
        self.write_top_module(emitted_design)?;

        self.end_element("design")
    }

    fn write_external_library(
        &mut self,
        design: &Design,
        include_capacitance: bool,
        external_lib: &str,
    ) -> Result<()> {
        let modules = if design.stage == "mapped" && external_lib == LOGICAL_EXTERNAL_LIB {
            mapped_external_modules(design)
        } else {
            collect_external_modules(design)
        };
        if modules.is_empty() {
            return Ok(());
        }

        let mut lib = BytesStart::new("external");
        lib.push_attribute(("name", external_lib));
        self.start_element(lib)?;
        for module in modules {
            self.write_external_module(&module, include_capacitance)?;
        }
        self.end_element("external")
    }

    fn write_external_module(
        &mut self,
        module: &ExternalModule,
        include_capacitance: bool,
    ) -> Result<()> {
        let mut element = BytesStart::new("module");
        element.push_attribute(("name", module.name.as_str()));
        element.push_attribute(("type", module.module_type.as_str()));
        self.start_element(element)?;
        for (name, value) in &module.properties {
            self.write_property(name, value, None)?;
        }
        for port in &module.ports {
            self.write_external_port(port, include_capacitance)?;
        }
        self.end_element("module")
    }

    fn write_external_port(
        &mut self,
        port: &ExternalPort,
        include_capacitance: bool,
    ) -> Result<()> {
        let mut element = BytesStart::new("port");
        element.push_attribute(("name", port.name.as_str()));
        element.push_attribute(("direction", port.direction.as_str()));
        if let Some(port_type) = port.port_type.as_deref() {
            element.push_attribute(("type", port_type));
        }
        if include_capacitance {
            element.push_attribute(("capacitance", "0.00000"));
        }
        self.empty_element(element)
    }

    fn write_work_library(
        &mut self,
        design: &Design,
        include_capacitance: bool,
        external_lib: &str,
    ) -> Result<()> {
        let mut lib = BytesStart::new("library");
        lib.push_attribute(("name", WORK_LIB));
        self.start_element(lib)?;

        let mut module = BytesStart::new("module");
        module.push_attribute(("name", design.name.as_str()));
        module.push_attribute(("type", "GENERIC"));
        self.start_element(module)?;
        for port in &design.ports {
            self.write_design_port(port, include_capacitance)?;
        }

        self.start_element(BytesStart::new("contents"))?;
        for cell in ordered_cells_for_write(design) {
            self.write_instance(cell, external_lib)?;
        }
        for net in &design.nets {
            self.write_net(net)?;
        }
        self.end_element("contents")?;

        self.end_element("module")?;
        self.end_element("library")
    }

    pub(super) fn write_design_port(
        &mut self,
        port: &Port,
        include_capacitance: bool,
    ) -> Result<()> {
        let mut properties = Vec::new();
        if let Some(pin) = port.pin.as_deref() {
            properties.push(("fde_pin", pin.to_string(), None));
        }
        if let (Some(x), Some(y)) = (port.x, port.y) {
            properties.push(("fde_position", format!("{x},{y}"), Some("point")));
        }
        self.write_port_with_properties(
            &port.name,
            port.direction.as_str(),
            port.width,
            include_capacitance,
            &properties,
        )
    }

    fn write_instance(&mut self, cell: &Cell, external_lib: &str) -> Result<()> {
        let mut element = BytesStart::new("instance");
        element.push_attribute(("name", cell.name.as_str()));
        element.push_attribute(("moduleRef", cell.type_name.as_str()));
        element.push_attribute(("libraryRef", external_lib));
        self.start_element(element)?;
        for property in &cell.properties {
            self.write_property(
                &fde_cell_property_name(property.key.as_str()),
                &fde_cell_property_value(cell, property.key.as_str(), &property.value),
                None,
            )?;
        }
        self.end_element("instance")
    }

    fn write_net(&mut self, net: &Net) -> Result<()> {
        let mut element = BytesStart::new("net");
        element.push_attribute(("name", net.name.as_str()));
        self.start_element(element)?;

        if let Some(driver) = &net.driver {
            self.write_port_ref(driver)?;
        }
        for sink in &net.sinks {
            self.write_port_ref(sink)?;
        }
        if net.estimated_delay_ns != 0.0 {
            self.write_property(
                "fde_estimated_delay_ns",
                &format!("{:.6}", net.estimated_delay_ns),
                None,
            )?;
        }
        if net.criticality != 0.0 {
            self.write_property("fde_criticality", &format!("{:.6}", net.criticality), None)?;
        }
        for property in &net.properties {
            self.write_property(&format!("fde_net_{}", property.key), &property.value, None)?;
        }
        self.write_route(&net.route, &net.route_pips)?;
        self.end_element("net")
    }

    fn write_port_ref(&mut self, endpoint: &Endpoint) -> Result<()> {
        let mut element = BytesStart::new("portRef");
        match endpoint.kind {
            crate::domain::EndpointKind::Cell => {
                element.push_attribute(("name", endpoint.pin.as_str()));
                element.push_attribute(("instanceRef", endpoint.name.as_str()));
            }
            crate::domain::EndpointKind::Port | crate::domain::EndpointKind::Unknown => {
                element.push_attribute(("name", endpoint.name.as_str()));
            }
        }
        self.empty_element(element)
    }

    fn write_route(&mut self, segments: &[RouteSegment], pips: &[RoutePip]) -> Result<()> {
        if !pips.is_empty() {
            self.write_pips(pips)
        } else {
            self.write_segments_as_properties(segments)
        }
    }

    fn write_segments_as_properties(&mut self, segments: &[RouteSegment]) -> Result<()> {
        for (index, segment) in segments.iter().enumerate() {
            self.write_property(
                &format!("fde_segment_{index:04}"),
                &format!(
                    "{},{},{},{}",
                    segment.x0, segment.y0, segment.x1, segment.y1
                ),
                Some("segment"),
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct ExternalModule {
    pub(super) name: String,
    pub(super) module_type: String,
    pub(super) properties: Vec<(String, String)>,
    pub(super) ports: Vec<ExternalPort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExternalPort {
    pub(super) name: String,
    pub(super) direction: PortDirection,
    pub(super) port_type: Option<String>,
}

struct MappedExternalModuleSpec {
    name: &'static str,
    module_type: &'static str,
    properties: &'static [(&'static str, &'static str)],
    ports: &'static [(&'static str, &'static str, Option<&'static str>)],
}

fn collect_external_modules(design: &Design) -> Vec<ExternalModule> {
    let mut modules = BTreeMap::<String, ExternalModule>::new();
    for cell in &design.cells {
        let module = modules
            .entry(cell.type_name.clone())
            .or_insert_with(|| ExternalModule {
                name: cell.type_name.clone(),
                module_type: module_type_name(cell),
                properties: external_module_properties(cell.type_name.as_str()),
                ports: Vec::new(),
            });
        let mut port_map = module
            .ports
            .iter()
            .cloned()
            .map(|port| (port.name.clone(), port))
            .collect::<BTreeMap<_, _>>();
        let primitive = cell.primitive_kind();
        for pin in &cell.inputs {
            port_map
                .entry(pin.port.clone())
                .or_insert_with(|| ExternalPort {
                    name: pin.port.clone(),
                    direction: PortDirection::Input,
                    port_type: primitive
                        .is_clock_pin(&pin.port)
                        .then(|| "clock".to_string()),
                });
        }
        for pin in &cell.outputs {
            port_map
                .entry(pin.port.clone())
                .or_insert_with(|| ExternalPort {
                    name: pin.port.clone(),
                    direction: PortDirection::Output,
                    port_type: None,
                });
        }
        if cell.type_name == "LOGIC_0" {
            port_map
                .entry("LOGIC_0_PIN".to_string())
                .or_insert_with(|| ExternalPort {
                    name: "LOGIC_0_PIN".to_string(),
                    direction: PortDirection::Output,
                    port_type: None,
                });
        }
        if cell.type_name == "LOGIC_1" {
            port_map
                .entry("LOGIC_1_PIN".to_string())
                .or_insert_with(|| ExternalPort {
                    name: "LOGIC_1_PIN".to_string(),
                    direction: PortDirection::Output,
                    port_type: None,
                });
        }
        module.ports = port_map.into_values().collect();
    }
    modules.into_values().collect()
}

fn ordered_cells_for_write(design: &Design) -> Vec<&Cell> {
    if design.stage != "mapped" {
        return design.cells.iter().collect();
    }

    let mut ordered = Vec::with_capacity(design.cells.len());
    let mut helpers = Vec::new();
    for cell in &design.cells {
        if let Some(rank) = mapped_helper_write_rank(cell.type_name.as_str()) {
            helpers.push((rank, cell));
        } else {
            ordered.push(cell);
        }
    }
    helpers.sort_by(|lhs, rhs| {
        lhs.0
            .cmp(&rhs.0)
            .then_with(|| lhs.1.name.cmp(&rhs.1.name))
            .then_with(|| lhs.1.type_name.cmp(&rhs.1.type_name))
    });
    ordered.extend(helpers.into_iter().map(|(_, cell)| cell));
    ordered
}

fn mapped_helper_write_rank(type_name: &str) -> Option<u8> {
    match type_name {
        "IBUF" => Some(0),
        "CLKBUF" => Some(1),
        "IOBUF" => Some(2),
        "OBUF" => Some(3),
        "IPAD" => Some(4),
        "OPAD" => Some(5),
        "LOGIC_1" => Some(6),
        "LOGIC_0" => Some(7),
        _ => None,
    }
}

fn mapped_external_modules(design: &Design) -> Vec<ExternalModule> {
    let used_modules = design
        .cells
        .iter()
        .map(|cell| cell.type_name.as_str())
        .collect::<BTreeSet<_>>();
    mapped_external_module_specs()
        .into_iter()
        .filter(|spec| used_modules.contains(spec.name))
        .map(mapped_external_module)
        .collect()
}

fn mapped_external_module_specs() -> Vec<MappedExternalModuleSpec> {
    let mut specs = Vec::new();
    specs.extend(mapped_lut_module_specs());
    specs.extend(mapped_ff_module_specs());
    specs.extend(mapped_io_module_specs());
    specs.extend(mapped_logic_constant_module_specs());
    specs
}

fn mapped_lut_module_specs() -> [MappedExternalModuleSpec; 4] {
    [
        MappedExternalModuleSpec {
            name: "LUT1",
            module_type: "LUT",
            properties: &[],
            ports: &[("ADR0", "input", None), ("O", "output", None)],
        },
        MappedExternalModuleSpec {
            name: "LUT2",
            module_type: "LUT",
            properties: &[],
            ports: &[
                ("ADR0", "input", None),
                ("ADR1", "input", None),
                ("O", "output", None),
            ],
        },
        MappedExternalModuleSpec {
            name: "LUT3",
            module_type: "LUT",
            properties: &[],
            ports: &[
                ("ADR0", "input", None),
                ("ADR1", "input", None),
                ("ADR2", "input", None),
                ("O", "output", None),
            ],
        },
        MappedExternalModuleSpec {
            name: "LUT4",
            module_type: "LUT",
            properties: &[],
            ports: &[
                ("ADR0", "input", None),
                ("ADR1", "input", None),
                ("ADR2", "input", None),
                ("ADR3", "input", None),
                ("O", "output", None),
            ],
        },
    ]
}

fn mapped_ff_module_specs() -> [MappedExternalModuleSpec; 2] {
    [
        MappedExternalModuleSpec {
            name: "DFFHQ",
            module_type: "FFLATCH",
            properties: &[("edge", "rise")],
            ports: &[
                ("D", "input", None),
                ("CK", "input", Some("clock")),
                ("Q", "output", None),
            ],
        },
        MappedExternalModuleSpec {
            name: "EDFFHQ",
            module_type: "FFLATCH",
            properties: &[("edge", "rise")],
            ports: &[
                ("D", "input", None),
                ("E", "input", None),
                ("CK", "input", Some("clock")),
                ("Q", "output", None),
            ],
        },
    ]
}

fn mapped_io_module_specs() -> [MappedExternalModuleSpec; 5] {
    [
        MappedExternalModuleSpec {
            name: "IBUF",
            module_type: "COMB",
            properties: &[("truthtable", "1")],
            ports: &[("I", "input", None), ("O", "output", None)],
        },
        MappedExternalModuleSpec {
            name: "CLKBUF",
            module_type: "COMB",
            properties: &[("truthtable", "1")],
            ports: &[("I", "input", None), ("O", "output", None)],
        },
        MappedExternalModuleSpec {
            name: "IPAD",
            module_type: "MACRO",
            properties: &[],
            ports: &[("PAD", "input", None)],
        },
        MappedExternalModuleSpec {
            name: "OBUF",
            module_type: "COMB",
            properties: &[("truthtable", "1")],
            ports: &[("I", "input", None), ("O", "output", None)],
        },
        MappedExternalModuleSpec {
            name: "OPAD",
            module_type: "MACRO",
            properties: &[],
            ports: &[("PAD", "output", None)],
        },
    ]
}

fn mapped_logic_constant_module_specs() -> [MappedExternalModuleSpec; 2] {
    [
        MappedExternalModuleSpec {
            name: "LOGIC_1",
            module_type: "MACRO",
            properties: &[("truthtable", "|1")],
            ports: &[("LOGIC_1_PIN", "output", None)],
        },
        MappedExternalModuleSpec {
            name: "LOGIC_0",
            module_type: "MACRO",
            properties: &[("truthtable", "|0")],
            ports: &[("LOGIC_0_PIN", "output", None)],
        },
    ]
}

fn mapped_external_module(spec: MappedExternalModuleSpec) -> ExternalModule {
    ExternalModule {
        name: spec.name.to_string(),
        module_type: spec.module_type.to_string(),
        properties: spec
            .properties
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        ports: spec.ports.iter().map(mapped_external_port).collect(),
    }
}

fn mapped_external_port(
    &(name, direction, port_type): &(&'static str, &'static str, Option<&'static str>),
) -> ExternalPort {
    ExternalPort {
        name: name.to_string(),
        direction: direction.parse().expect("mapped port direction"),
        port_type: port_type.map(str::to_string),
    }
}

fn module_type_name(cell: &Cell) -> String {
    match cell.type_name.as_str() {
        "IBUF" | "CLKBUF" | "OBUF" => return "COMB".to_string(),
        "IPAD" | "OPAD" | "LOGIC_1" | "LOGIC_0" | "VCC" | "GND" => {
            return "MACRO".to_string();
        }
        _ => {}
    }
    match cell.kind {
        CellKind::Lut => "LUT".to_string(),
        CellKind::Ff | CellKind::Latch => "FFLATCH".to_string(),
        CellKind::BlockRam => "BRAM".to_string(),
        CellKind::Io => "IO".to_string(),
        CellKind::GlobalClockBuffer => "GCLK".to_string(),
        CellKind::Constant => "CONST".to_string(),
        CellKind::Buffer => "COMB".to_string(),
        CellKind::Generic | CellKind::Unknown => match cell.primitive_kind() {
            PrimitiveKind::GlobalClockBuffer => "GCLK".to_string(),
            PrimitiveKind::Buffer => "COMB".to_string(),
            PrimitiveKind::Io => "IO".to_string(),
            PrimitiveKind::Lut { .. } => "LUT".to_string(),
            PrimitiveKind::FlipFlop | PrimitiveKind::Latch => "FFLATCH".to_string(),
            PrimitiveKind::BlockRam => "BRAM".to_string(),
            PrimitiveKind::Constant(_) => "CONST".to_string(),
            PrimitiveKind::Generic | PrimitiveKind::Unknown => "GENERIC".to_string(),
        },
    }
}

fn external_module_properties(type_name: &str) -> Vec<(String, String)> {
    match type_name {
        "DFFHQ" | "EDFFHQ" => vec![("edge".to_string(), "rise".to_string())],
        "IBUF" | "CLKBUF" | "OBUF" => vec![("truthtable".to_string(), "1".to_string())],
        "LOGIC_1" | "VCC" => vec![("truthtable".to_string(), "|1".to_string())],
        "LOGIC_0" | "GND" => vec![("truthtable".to_string(), "|0".to_string())],
        _ => Vec::new(),
    }
}

pub(super) fn fde_cell_property_name(key: &str) -> String {
    if key.eq_ignore_ascii_case("lut_init") {
        "INIT".to_string()
    } else {
        key.to_string()
    }
}

pub(super) fn fde_cell_property_value(cell: &Cell, key: &str, value: &str) -> String {
    if (key.eq_ignore_ascii_case("lut_init") || key.eq_ignore_ascii_case("init")) && cell.is_lut() {
        if let Some(bits) = logical_lut_truth_table_bits(cell) {
            return format_truth_table_literal(&bits)
                .trim_start_matches("0x")
                .to_string();
        }
        return value
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X")
            .to_string();
    }
    value.to_string()
}

pub(super) fn is_clock_input_port(design: &Design, port_name: &str) -> bool {
    design.nets.iter().any(|net| {
        net.driver.as_ref().is_some_and(|driver| {
            driver.kind == crate::domain::EndpointKind::Port && driver.name == port_name
        }) && net.sinks.iter().any(|sink| {
            design
                .cells
                .iter()
                .find(|cell| cell.name == sink.name)
                .is_some_and(|cell| cell.primitive_kind().is_clock_pin(&sink.pin))
        })
    })
}

fn logical_lut_truth_table_bits(cell: &Cell) -> Option<Vec<u8>> {
    let logical_bits = logical_truth_table_bits(cell.primitive_kind())?;
    if let Some(bits) = cell
        .property("init")
        .and_then(|init| parse_compact_hex_digit_literal(init, logical_bits))
    {
        return Some(bits);
    }

    cell.property("lut_init")
        .and_then(|init| parse_bit_literal(init, logical_bits))
}

pub(super) fn packed_lut_function_name(cell: &Cell) -> Option<String> {
    let bits = logical_lut_truth_table_bits(cell)?;
    let input_count = logical_lut_input_count(cell.primitive_kind())?;
    Some(encode_lut_expression_literal(&bits, input_count))
}

fn logical_lut_input_count(primitive: PrimitiveKind) -> Option<usize> {
    match primitive {
        PrimitiveKind::Lut {
            inputs: Some(inputs),
        } => Some(inputs),
        PrimitiveKind::Lut { inputs: None }
        | PrimitiveKind::FlipFlop
        | PrimitiveKind::Latch
        | PrimitiveKind::Constant(_)
        | PrimitiveKind::Buffer
        | PrimitiveKind::Io
        | PrimitiveKind::GlobalClockBuffer
        | PrimitiveKind::BlockRam
        | PrimitiveKind::Generic
        | PrimitiveKind::Unknown => None,
    }
}

fn logical_truth_table_bits(primitive: PrimitiveKind) -> Option<usize> {
    let inputs = match primitive {
        PrimitiveKind::Lut {
            inputs: Some(inputs),
        } => inputs,
        PrimitiveKind::Lut { inputs: None }
        | PrimitiveKind::FlipFlop
        | PrimitiveKind::Latch
        | PrimitiveKind::Constant(_)
        | PrimitiveKind::Buffer
        | PrimitiveKind::Io
        | PrimitiveKind::GlobalClockBuffer
        | PrimitiveKind::BlockRam
        | PrimitiveKind::Generic
        | PrimitiveKind::Unknown => return None,
    };
    1usize.checked_shl(inputs as u32)
}

fn format_truth_table_literal(bits: &[u8]) -> String {
    let digit_count = bits.len().max(1).div_ceil(4);
    let mut digits = String::with_capacity(digit_count);
    for digit_index in (0..digit_count).rev() {
        let nibble = (0..4).fold(0u8, |value, bit_index| {
            let bit = bits.get(digit_index * 4 + bit_index).copied().unwrap_or(0) & 1;
            value | (bit << bit_index)
        });
        digits.push(match nibble {
            0..=9 => char::from(b'0' + nibble),
            10..=15 => char::from(b'A' + (nibble - 10)),
            _ => '0',
        });
    }
    format!("0x{digits}")
}

pub(super) fn pin_map_indices(cell: &Cell, logical_index: usize) -> Vec<usize> {
    let key = pin_map_property_name(logical_index);
    let Some(value) = cell.property(&key) else {
        return vec![logical_index];
    };
    let mut indices = value
        .split(',')
        .filter_map(|entry| entry.trim().parse::<usize>().ok())
        .collect::<Vec<_>>();
    if indices.is_empty() {
        indices.push(logical_index);
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}

#[cfg(test)]
mod tests;
