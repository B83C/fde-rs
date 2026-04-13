use anyhow::Result;
use quick_xml::events::{BytesDecl, BytesStart, Event};
use std::path::Path;

use super::{
    CIL_PROPERTY, DesignXmlWriter, GCLK_PORTS, GCLKIOB_PORTS, IOB_PORTS, PHYSICAL_EXTERNAL_LIB,
    PhysicalDesignView, PhysicalInstance, PhysicalNet, SLICE_PORTS, WORK_LIB, XmlWriteContext,
    blockram_ports,
};
use crate::ir::Design;

impl DesignXmlWriter {
    pub(super) fn write_physical_design(
        &mut self,
        design: &Design,
        context: &XmlWriteContext<'_>,
        physical: &PhysicalDesignView,
    ) -> Result<()> {
        self.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

        let mut root = BytesStart::new("design");
        root.push_attribute(("name", design.name.as_str()));
        self.start_element(root)?;

        self.write_root_properties(context.cil_path)?;
        self.write_physical_external_library(physical.include_capacitance, &physical.used_modules)?;
        self.write_physical_work_library(design, physical)?;
        self.write_top_module(design)?;

        self.end_element("design")
    }

    pub(super) fn write_root_properties(&mut self, cil_path: Option<&Path>) -> Result<()> {
        if let Some(cil_path) = cil_path {
            let value = cil_path.display().to_string();
            self.write_property(CIL_PROPERTY, &value, None)?;
        }
        Ok(())
    }

    fn write_physical_external_library(
        &mut self,
        include_capacitance: bool,
        used_modules: &std::collections::BTreeSet<&'static str>,
    ) -> Result<()> {
        let mut lib = BytesStart::new("external");
        lib.push_attribute(("name", PHYSICAL_EXTERNAL_LIB));
        self.start_element(lib)?;
        self.write_physical_external_module(
            include_capacitance,
            used_modules,
            "blockram",
            "BLOCKRAM",
            &blockram_ports(),
        )?;
        self.write_physical_external_module(
            include_capacitance,
            used_modules,
            "slice",
            "SLICE",
            SLICE_PORTS,
        )?;
        self.write_physical_external_module(
            include_capacitance,
            used_modules,
            "iob",
            "IOB",
            IOB_PORTS,
        )?;
        self.write_physical_external_module(
            include_capacitance,
            used_modules,
            "gclk",
            "GCLK",
            GCLK_PORTS,
        )?;
        self.write_physical_external_module(
            include_capacitance,
            used_modules,
            "gclkiob",
            "GCLKIOB",
            GCLKIOB_PORTS,
        )?;
        self.end_element("external")
    }

    fn write_physical_external_module(
        &mut self,
        include_capacitance: bool,
        used_modules: &std::collections::BTreeSet<&'static str>,
        name: &'static str,
        module_type: &'static str,
        ports: &[super::PhysicalPortDesc],
    ) -> Result<()> {
        if !used_modules.contains(name) {
            return Ok(());
        }
        let mut module = BytesStart::new("module");
        module.push_attribute(("name", name));
        module.push_attribute(("type", module_type));
        self.start_element(module)?;
        for port in ports {
            self.write_xml_port(&port.name, port.direction, 1, include_capacitance, &[])?;
        }
        self.end_element("module")
    }

    fn write_physical_work_library(
        &mut self,
        design: &Design,
        physical: &PhysicalDesignView,
    ) -> Result<()> {
        let mut lib = BytesStart::new("library");
        lib.push_attribute(("name", WORK_LIB));
        self.start_element(lib)?;

        let mut module = BytesStart::new("module");
        module.push_attribute(("name", design.name.as_str()));
        module.push_attribute(("type", "GENERIC"));
        self.start_element(module)?;
        for port in &design.ports {
            self.write_design_port(port, physical.include_capacitance)?;
        }

        self.start_element(BytesStart::new("contents"))?;
        for instance in &physical.instances {
            self.write_physical_instance(instance)?;
        }
        for net in &physical.nets {
            self.write_physical_net(net)?;
        }
        self.end_element("contents")?;

        self.end_element("module")?;
        self.end_element("library")
    }

    fn write_physical_instance(&mut self, instance: &PhysicalInstance) -> Result<()> {
        let mut element = BytesStart::new("instance");
        element.push_attribute(("name", instance.name.as_str()));
        element.push_attribute(("moduleRef", instance.module_ref));
        element.push_attribute(("libraryRef", PHYSICAL_EXTERNAL_LIB));
        self.start_element(element)?;
        if let Some((x, y, z)) = instance.position {
            self.write_property("position", &format!("{x},{y},{z}"), Some("point"))?;
        }
        for (name, value) in &instance.configs {
            let mut config = BytesStart::new("config");
            config.push_attribute(("name", name.as_str()));
            config.push_attribute(("value", value.as_str()));
            self.empty_element(config)?;
        }
        self.end_element("instance")
    }

    fn write_physical_net(&mut self, net: &PhysicalNet) -> Result<()> {
        let mut element = BytesStart::new("net");
        element.push_attribute(("name", net.name.as_str()));
        if let Some(net_type) = net.net_type {
            element.push_attribute(("type", net_type));
        }
        self.start_element(element)?;
        for endpoint in &net.endpoints {
            let mut port_ref = BytesStart::new("portRef");
            port_ref.push_attribute(("name", endpoint.pin.as_str()));
            if let Some(instance_ref) = endpoint.instance_ref.as_deref() {
                port_ref.push_attribute(("instanceRef", instance_ref));
            }
            self.empty_element(port_ref)?;
        }
        self.write_pips(&net.pips)?;
        self.end_element("net")
    }
}
