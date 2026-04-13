mod logical;
mod mapped;
mod physical_xml;

use crate::{
    cil::Cil,
    constraints::ConstraintEntry,
    ir::{Design, RoutePip},
    resource::Arch,
};
use anyhow::{Context, Result};
use quick_xml::{
    Writer,
    events::{BytesEnd, BytesStart, Event},
};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mapped::build_fde_mapped_design;

const WORK_LIB: &str = "work_lib";
const LOGICAL_EXTERNAL_LIB: &str = "cell_lib";
const PHYSICAL_EXTERNAL_LIB: &str = "template_work_lib";
const CIL_PROPERTY: &str = "cil_fname";

const SLICE_PORTS: &[PhysicalPortDesc] = &[
    PhysicalPortDesc::new("CIN", "input"),
    PhysicalPortDesc::new("SR", "input"),
    PhysicalPortDesc::new("CLK", "input"),
    PhysicalPortDesc::new("CE", "input"),
    PhysicalPortDesc::new("BX", "input"),
    PhysicalPortDesc::new("F1", "input"),
    PhysicalPortDesc::new("F2", "input"),
    PhysicalPortDesc::new("F3", "input"),
    PhysicalPortDesc::new("F4", "input"),
    PhysicalPortDesc::new("F5IN", "input"),
    PhysicalPortDesc::new("BY", "input"),
    PhysicalPortDesc::new("G1", "input"),
    PhysicalPortDesc::new("G2", "input"),
    PhysicalPortDesc::new("G3", "input"),
    PhysicalPortDesc::new("G4", "input"),
    PhysicalPortDesc::new("XQ", "output"),
    PhysicalPortDesc::new("X", "output"),
    PhysicalPortDesc::new("F5", "output"),
    PhysicalPortDesc::new("XB", "output"),
    PhysicalPortDesc::new("YQ", "output"),
    PhysicalPortDesc::new("Y", "output"),
    PhysicalPortDesc::new("YB", "output"),
    PhysicalPortDesc::new("COUT", "output"),
];

const IOB_PORTS: &[PhysicalPortDesc] = &[
    PhysicalPortDesc::new("TRI", "input"),
    PhysicalPortDesc::new("TRICE", "input"),
    PhysicalPortDesc::new("OUT", "input"),
    PhysicalPortDesc::new("OUTCE", "input"),
    PhysicalPortDesc::new("INCE", "input"),
    PhysicalPortDesc::new("CLK", "input"),
    PhysicalPortDesc::new("SR", "input"),
    PhysicalPortDesc::new("IN", "output"),
    PhysicalPortDesc::new("IQ", "output"),
    PhysicalPortDesc::new("PAD", "inout"),
];

const GCLK_PORTS: &[PhysicalPortDesc] = &[
    PhysicalPortDesc::new("CE", "input"),
    PhysicalPortDesc::new("IN", "input"),
    PhysicalPortDesc::new("OUT", "output"),
];

const GCLKIOB_PORTS: &[PhysicalPortDesc] = &[
    PhysicalPortDesc::new("PAD", "inout"),
    PhysicalPortDesc::new("GCLKOUT", "output"),
];

fn blockram_ports() -> Vec<PhysicalPortDesc> {
    let mut ports = vec![
        PhysicalPortDesc::new("CKA", "input"),
        PhysicalPortDesc::new("AWE", "input"),
        PhysicalPortDesc::new("AEN", "input"),
        PhysicalPortDesc::new("RSTA", "input"),
        PhysicalPortDesc::new("CKB", "input"),
        PhysicalPortDesc::new("BWE", "input"),
        PhysicalPortDesc::new("BEN", "input"),
        PhysicalPortDesc::new("RSTB", "input"),
    ];
    for index in 0..12 {
        ports.push(PhysicalPortDesc::owned(format!("ADDRA_{index}"), "input"));
    }
    for index in 0..12 {
        ports.push(PhysicalPortDesc::owned(format!("ADDRB_{index}"), "input"));
    }
    for index in 0..16 {
        ports.push(PhysicalPortDesc::owned(format!("DINA{index}"), "input"));
    }
    for index in 0..16 {
        ports.push(PhysicalPortDesc::owned(format!("DINB{index}"), "input"));
    }
    for index in 0..16 {
        ports.push(PhysicalPortDesc::owned(format!("DOUTA{index}"), "output"));
    }
    for index in 0..16 {
        ports.push(PhysicalPortDesc::owned(format!("DOUTB{index}"), "output"));
    }
    ports
}

pub(super) const SLICE_DEFAULT_CONFIGS: &[(&str, &str)] = &[
    ("BXMUX", "#OFF"),
    ("BYMUX", "#OFF"),
    ("CEMUX", "#OFF"),
    ("CKINV", "#OFF"),
    ("COUTUSED", "#OFF"),
    ("CY0F", "#OFF"),
    ("CY0G", "#OFF"),
    ("CYINIT", "#OFF"),
    ("CYSELF", "#OFF"),
    ("CYSELG", "#OFF"),
    ("DXMUX", "#OFF"),
    ("DYMUX", "#OFF"),
    ("F", "#OFF"),
    ("F5USED", "#OFF"),
    ("FFX", "#OFF"),
    ("FFY", "#OFF"),
    ("FXMUX", "#OFF"),
    ("G", "#OFF"),
    ("GYMUX", "#OFF"),
    ("INITX", "#OFF"),
    ("INITY", "#OFF"),
    ("RAMCONFIG", "#OFF"),
    ("REVUSED", "#OFF"),
    ("SRFFMUX", "#OFF"),
    ("SRMUX", "#OFF"),
    ("SYNC_ATTR", "#OFF"),
    ("XBUSED", "#OFF"),
    ("XUSED", "#OFF"),
    ("YBMUX", "#OFF"),
    ("YUSED", "#OFF"),
];

pub(super) const IOB_DEFAULT_CONFIGS: &[(&str, &str)] = &[
    ("DRIVEATTRBOX", "#OFF"),
    ("FFATTRBOX", "#OFF"),
    ("ICEMUX", "#OFF"),
    ("ICKINV", "#OFF"),
    ("IFF", "#OFF"),
    ("IFFINITATTR", "#OFF"),
    ("IFFMUX", "#OFF"),
    ("IINITMUX", "#OFF"),
    ("IMUX", "#OFF"),
    ("IOATTRBOX", "LVTTL"),
    ("OCEMUX", "#OFF"),
    ("OCKINV", "#OFF"),
    ("OFF", "#OFF"),
    ("OFFATTRBOX", "#OFF"),
    ("OINITMUX", "#OFF"),
    ("OMUX", "#OFF"),
    ("OUTMUX", "#OFF"),
    ("PULL", "#OFF"),
    ("SLEW", "#OFF"),
    ("SRMUX", "#OFF"),
    ("TCEMUX", "#OFF"),
    ("TCKINV", "#OFF"),
    ("TFF", "#OFF"),
    ("TFFATTRBOX", "#OFF"),
    ("TINITMUX", "#OFF"),
    ("TRIMUX", "#OFF"),
    ("TSEL", "#OFF"),
];

pub(super) const GCLK_DEFAULT_CONFIGS: &[(&str, &str)] = &[
    ("CEMUX", "1"),
    ("DISABLE_ATTR", "LOW"),
    ("CE_POWER", "#OFF"),
    ("GCLK_BUFFER", "#OFF"),
];

pub(super) const GCLKIOB_DEFAULT_CONFIGS: &[(&str, &str)] = &[
    ("IOATTRBOX", "LVTTL"),
    ("GCLK_BUF", "#OFF"),
    ("PAD", "#OFF"),
];

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct XmlWriteContext<'a> {
    pub(super) arch: Option<&'a Arch>,
    pub(super) _cil: Option<&'a Cil>,
    pub(super) _constraints: &'a [ConstraintEntry],
    pub(super) cil_path: Option<&'a Path>,
}

pub(super) fn save_design_xml(design: &Design, context: &XmlWriteContext<'_>) -> Result<String> {
    let mut writer = DesignXmlWriter::new();
    if let Some(mapped) = build_fde_mapped_design(design) {
        writer.write_logical_design(&mapped, LOGICAL_EXTERNAL_LIB)?;
    } else if let Some(physical) = PhysicalDesignView::build(design, context) {
        writer.write_physical_design(design, context, &physical)?;
    } else {
        writer.write_logical_design(design, LOGICAL_EXTERNAL_LIB)?;
    }
    writer.finish()
}

pub(super) fn is_clock_input_port(design: &Design, port_name: &str) -> bool {
    logical::is_clock_input_port(design, port_name)
}

pub(super) fn physical_lut_function_name(cell: &crate::ir::Cell) -> Option<String> {
    logical::physical_lut_function_name(cell)
}

pub(super) fn pin_map_indices(cell: &crate::ir::Cell, logical_index: usize) -> Vec<usize> {
    logical::pin_map_indices(cell, logical_index)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhysicalPortDesc {
    name: Cow<'static, str>,
    direction: &'static str,
}

impl PhysicalPortDesc {
    const fn new(name: &'static str, direction: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            direction,
        }
    }

    fn owned(name: String, direction: &'static str) -> Self {
        Self {
            name: Cow::Owned(name),
            direction,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PhysicalDesignView {
    pub(crate) instances: Vec<PhysicalInstance>,
    pub(crate) nets: Vec<PhysicalNet>,
    pub(crate) used_modules: BTreeSet<&'static str>,
    pub(crate) include_capacitance: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PhysicalInstance {
    pub(crate) name: String,
    pub(crate) module_ref: &'static str,
    pub(crate) position: Option<(usize, usize, usize)>,
    pub(crate) configs: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct PhysicalNet {
    pub(crate) name: String,
    pub(crate) net_type: Option<&'static str>,
    pub(crate) endpoints: Vec<PhysicalEndpoint>,
    pub(crate) pips: Vec<RoutePip>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PhysicalEndpoint {
    pub(crate) pin: String,
    pub(crate) instance_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SliceCellKind {
    Lut,
    Sequential,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SliceCellBinding {
    pub(super) slot: usize,
    pub(super) kind: SliceCellKind,
}

#[derive(Debug, Clone)]
pub(super) struct PortInstanceBinding {
    pub(super) port_name: String,
    pub(super) input_used: bool,
    pub(super) output_used: bool,
    pub(super) pad_instance_name: String,
    pub(super) pad_module_ref: &'static str,
    pub(super) pad_position: Option<(usize, usize, usize)>,
    pub(super) gclk_instance_name: Option<String>,
    pub(super) gclk_position: Option<(usize, usize, usize)>,
    pub(super) clock_input: bool,
    pub(super) tile_wire_prefix: Option<String>,
    pub(super) tile_position: Option<(usize, usize, usize)>,
}

pub(super) struct DesignXmlWriter {
    writer: Writer<Vec<u8>>,
}

impl DesignXmlWriter {
    fn new() -> Self {
        Self {
            writer: Writer::new_with_indent(Vec::new(), b' ', 2),
        }
    }

    fn finish(self) -> Result<String> {
        String::from_utf8(self.writer.into_inner()).context("design xml is not valid utf-8")
    }

    fn write_port_with_properties(
        &mut self,
        name: &str,
        direction: &str,
        width: usize,
        include_capacitance: bool,
        properties: &[(impl AsRef<str>, impl AsRef<str>, Option<&str>)],
    ) -> Result<()> {
        let mut element = BytesStart::new("port");
        element.push_attribute(("name", name));
        element.push_attribute(("direction", direction));
        if width > 1 {
            let msb = (width - 1).to_string();
            element.push_attribute(("msb", msb.as_str()));
            element.push_attribute(("lsb", "0"));
        }
        if include_capacitance {
            element.push_attribute(("capacitance", "0.00000"));
        }
        if properties.is_empty() {
            self.empty_element(element)
        } else {
            self.start_element(element)?;
            for (name, value, value_type) in properties {
                self.write_property(name.as_ref(), value.as_ref(), *value_type)?;
            }
            self.end_element("port")
        }
    }

    fn write_xml_port(
        &mut self,
        name: &str,
        direction: &str,
        width: usize,
        include_capacitance: bool,
        properties: &[(&str, &str, Option<&str>)],
    ) -> Result<()> {
        self.write_port_with_properties(name, direction, width, include_capacitance, properties)
    }

    fn write_pips(&mut self, pips: &[RoutePip]) -> Result<()> {
        for pip in pips {
            let position = format!("{},{}", pip.x, pip.y);
            let mut element = BytesStart::new("pip");
            element.push_attribute(("from", pip.from_net.as_str()));
            element.push_attribute(("to", pip.to_net.as_str()));
            element.push_attribute(("position", position.as_str()));
            element.push_attribute(("dir", "->"));
            self.empty_element(element)?;
        }
        Ok(())
    }

    fn write_top_module(&mut self, design: &Design) -> Result<()> {
        let mut top = BytesStart::new("topModule");
        top.push_attribute(("name", design.name.as_str()));
        top.push_attribute(("libraryRef", WORK_LIB));
        self.empty_element(top)
    }

    fn write_property(&mut self, name: &str, value: &str, value_type: Option<&str>) -> Result<()> {
        let mut property = BytesStart::new("property");
        property.push_attribute(("name", name));
        if let Some(value_type) = value_type {
            property.push_attribute(("type", value_type));
        }
        property.push_attribute(("value", value));
        self.empty_element(property)
    }

    fn start_element(&mut self, element: BytesStart<'_>) -> Result<()> {
        self.write_event(Event::Start(element))
    }

    fn empty_element(&mut self, element: BytesStart<'_>) -> Result<()> {
        self.write_event(Event::Empty(element))
    }

    fn end_element(&mut self, tag: &str) -> Result<()> {
        self.write_event(Event::End(BytesEnd::new(tag)))
    }

    fn write_event<'a>(&mut self, event: Event<'a>) -> Result<()> {
        self.writer
            .write_event(event)
            .context("failed to serialize design xml")
    }
}

pub(super) fn default_configs(defaults: &[(&str, &str)]) -> Vec<(String, String)> {
    defaults
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

pub(super) fn default_config_map(defaults: &[(&str, &str)]) -> BTreeMap<String, String> {
    defaults
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

pub(super) fn ordered_configs(
    defaults: &[(&str, &str)],
    values: BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut ordered = Vec::new();
    for (name, _) in defaults {
        if let Some(value) = values.get(*name) {
            ordered.push(((*name).to_string(), value.clone()));
        }
    }
    ordered
}
