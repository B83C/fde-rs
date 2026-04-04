use super::{mapped_external_modules, ordered_cells_for_write, packed_lut_function_name};
use crate::{
    infra::io::xml::writer::{XmlWriteContext, save_design_xml},
    ir::{Cell, Design, Endpoint, Net, Port, Property},
};

#[test]
fn packed_lut_function_preserves_raw_init_width_for_lut3() {
    let mut cell = Cell::lut("u0", "LUT3");
    cell.properties.push(Property::new("INIT", "01"));
    assert_eq!(
        packed_lut_function_name(&cell).as_deref(),
        Some("#LUT:D=((~A3*~A2)*~A1)")
    );
}

#[test]
fn packed_lut_function_prefers_raw_init_compact_hex_semantics() {
    let mut cell = Cell::lut("u0", "LUT3");
    cell.properties.push(Property::new("init", "78"));
    cell.properties.push(Property::new("lut_init", "0x4E"));
    assert_eq!(
        packed_lut_function_name(&cell).as_deref(),
        Some("#LUT:D=((A3*A2)*~A1)+((A3*~A2)*A1)+((A3*~A2)*~A1)+((~A3*A2)*A1)")
    );
}

#[test]
fn mapped_external_modules_define_edffhq_when_used() {
    let design = Design {
        stage: "mapped".to_string(),
        cells: vec![
            Cell::ff("ff0", "EDFFHQ")
                .with_input("D", "d")
                .with_input("E", "ce")
                .with_input("CK", "clk")
                .with_output("Q", "q"),
        ],
        ..Design::default()
    };

    let modules = mapped_external_modules(&design);
    let edff = modules
        .iter()
        .find(|module| module.name == "EDFFHQ")
        .expect("EDFFHQ module definition");

    assert_eq!(edff.module_type, "FFLATCH");
    assert!(
        edff.properties
            .iter()
            .any(|(key, value)| key == "edge" && value == "rise")
    );
    assert!(edff.ports.iter().any(|port| port.name == "E"));
    assert!(
        edff.ports
            .iter()
            .any(|port| port.name == "CK" && port.port_type.as_deref() == Some("clock"))
    );
}

#[test]
fn mapped_xml_canonicalizes_helper_instance_order_for_cpp_pack() {
    let design = Design {
        name: "helper_order".to_string(),
        stage: "mapped".to_string(),
        ports: vec![
            Port::input("rst"),
            Port::input("clk"),
            Port::input("ena"),
            Port::output("z_out"),
            Port::output("a_out"),
        ],
        cells: vec![
            Cell::ff("ff0", "EDFFHQ")
                .with_input("D", "next_q")
                .with_input("E", "ena")
                .with_input("CK", "clk")
                .with_output("Q", "q"),
            Cell::new("lut0", crate::domain::CellKind::Lut, "LUT1")
                .with_input("ADR0", "rst")
                .with_output("O", "next_q"),
            Cell::new("lut1", crate::domain::CellKind::Lut, "LUT1")
                .with_input("ADR0", "q")
                .with_output("O", "a_out"),
            Cell::new("lut2", crate::domain::CellKind::Lut, "LUT1")
                .with_input("ADR0", "ena")
                .with_output("O", "z_out"),
        ],
        nets: vec![
            Net::new("rst")
                .with_driver(Endpoint::port("rst", "rst"))
                .with_sink(Endpoint::cell("lut0", "ADR0")),
            Net::new("clk")
                .with_driver(Endpoint::port("clk", "clk"))
                .with_sink(Endpoint::cell("ff0", "CK")),
            Net::new("ena")
                .with_driver(Endpoint::port("ena", "ena"))
                .with_sink(Endpoint::cell("ff0", "E"))
                .with_sink(Endpoint::cell("lut2", "ADR0")),
            Net::new("next_q")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("q")
                .with_driver(Endpoint::cell("ff0", "Q"))
                .with_sink(Endpoint::cell("lut1", "ADR0")),
            Net::new("a_out")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::port("a_out", "a_out")),
            Net::new("z_out")
                .with_driver(Endpoint::cell("lut2", "O"))
                .with_sink(Endpoint::port("z_out", "z_out")),
        ],
        ..Design::default()
    };

    let mapped = super::super::mapped::build_fde_mapped_design(&design).expect("mapped design");
    let ordered = ordered_cells_for_write(&mapped)
        .into_iter()
        .map(|cell| (cell.name.as_str(), cell.type_name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        ordered[4..],
        vec![
            ("Buf-pad-ena", "IBUF"),
            ("Buf-pad-rst", "IBUF"),
            ("Buf-pad-clk", "CLKBUF"),
            ("IBuf-clkpad-clk", "CLKBUF"),
            ("Buf-pad-a_out", "OBUF"),
            ("Buf-pad-z_out", "OBUF"),
            ("clk_ipad", "IPAD"),
            ("ena_ipad", "IPAD"),
            ("rst_ipad", "IPAD"),
            ("a_out_opad", "OPAD"),
            ("z_out_opad", "OPAD"),
        ]
    );

    let xml = save_design_xml(&design, &XmlWriteContext::default()).expect("mapped xml");
    let doc = roxmltree::Document::parse(&xml).expect("xml document");
    let instances = doc
        .descendants()
        .filter(|node| node.has_tag_name("instance"))
        .map(|node| {
            (
                node.attribute("name").expect("instance name"),
                node.attribute("moduleRef").expect("module ref"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        &instances[4..15],
        &[
            ("Buf-pad-ena", "IBUF"),
            ("Buf-pad-rst", "IBUF"),
            ("Buf-pad-clk", "CLKBUF"),
            ("IBuf-clkpad-clk", "CLKBUF"),
            ("Buf-pad-a_out", "OBUF"),
            ("Buf-pad-z_out", "OBUF"),
            ("clk_ipad", "IPAD"),
            ("ena_ipad", "IPAD"),
            ("rst_ipad", "IPAD"),
            ("a_out_opad", "OPAD"),
            ("z_out_opad", "OPAD"),
        ]
    );
}
