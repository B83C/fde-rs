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

#[test]
fn mapped_lut_init_with_leading_zero_b_is_still_imported_as_hex() {
    let xml = r#"
<design name="demo">
  <external name="cell_lib">
    <module name="LUT4" type="LUT">
      <port name="A" direction="input"/>
      <port name="B" direction="input"/>
      <port name="C" direction="input"/>
      <port name="D" direction="input"/>
      <property name="INIT" value="0000"/>
      <port name="O" direction="output"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="demo" type="GENERIC">
      <port name="y" direction="output"/>
      <contents>
        <instance name="u0" moduleRef="LUT4" libraryRef="cell_lib">
          <property name="INIT" value="0B00"/>
        </instance>
        <net name="y">
          <portRef name="O" instanceRef="u0"/>
          <portRef name="y"/>
        </net>
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
        .find(|cell| cell.name == "u0")
        .expect("lut cell");
    assert_eq!(cell.property("lut_init"), Some("0x0B00"));
}

#[test]
fn mapped_import_expands_bus_ports_into_bit_ports() {
    let xml = r#"
<design name="demo">
  <external name="cell_lib">
    <module name="OBUF" type="OBUF">
      <port name="I" direction="input"/>
      <port name="O" direction="output"/>
    </module>
    <module name="OPAD" type="OPAD">
      <port name="PAD" direction="output"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="demo" type="GENERIC">
      <port name="led" msb="1" lsb="0" direction="output"/>
      <contents>
        <instance name="Buf-pad-led[1]" moduleRef="OBUF" libraryRef="cell_lib"/>
        <instance name="led[1]_opad" moduleRef="OPAD" libraryRef="cell_lib"/>
        <instance name="Buf-pad-led[0]" moduleRef="OBUF" libraryRef="cell_lib"/>
        <instance name="led[0]_opad" moduleRef="OPAD" libraryRef="cell_lib"/>
        <net name="net_Buf-pad-led[1]">
          <portRef name="O" instanceRef="Buf-pad-led[1]"/>
          <portRef name="led[1]"/>
        </net>
        <net name="led[1]">
          <portRef name="PAD" instanceRef="led[1]_opad"/>
          <portRef name="I" instanceRef="Buf-pad-led[1]"/>
          <portRef name="led[1]"/>
        </net>
        <net name="net_Buf-pad-led[0]">
          <portRef name="O" instanceRef="Buf-pad-led[0]"/>
          <portRef name="led[0]"/>
        </net>
        <net name="led[0]">
          <portRef name="PAD" instanceRef="led[0]_opad"/>
          <portRef name="I" instanceRef="Buf-pad-led[0]"/>
          <portRef name="led[0]"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule name="demo" libraryRef="work_lib"/>
</design>
"#;

    let doc = Document::parse(xml).expect("xml parse");
    let design = load_fde_mapped_design_xml(doc.root_element()).expect("mapped xml import");

    let port_names = design
        .ports
        .iter()
        .map(|port| port.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(port_names, vec!["led[1]", "led[0]"]);

    let net_names = design
        .nets
        .iter()
        .map(|net| net.name.as_str())
        .collect::<Vec<_>>();
    assert!(net_names.contains(&"led[1]"));
    assert!(net_names.contains(&"led[0]"));
}

#[test]
fn mapped_import_lowers_constant_output_drivers_into_lut_cells() {
    let xml = r#"
<design name="const_zero_output">
  <external name="cell_lib">
    <module name="LOGIC_0" type="LOGIC_0">
      <port name="LOGIC_0_PIN" direction="output"/>
    </module>
    <module name="OBUF" type="OBUF">
      <port name="I" direction="input"/>
      <port name="O" direction="output"/>
    </module>
    <module name="OPAD" type="OPAD">
      <port name="PAD" direction="output"/>
    </module>
  </external>
  <library name="work_lib">
    <module name="const_zero_output" type="GENERIC">
      <port name="led" direction="output"/>
      <contents>
        <instance name="GND" moduleRef="LOGIC_0" libraryRef="cell_lib"/>
        <instance name="Buf-pad-led" moduleRef="OBUF" libraryRef="cell_lib"/>
        <instance name="led_opad" moduleRef="OPAD" libraryRef="cell_lib"/>
        <net name="net_Buf-pad-led">
          <portRef name="LOGIC_0_PIN" instanceRef="GND"/>
          <portRef name="I" instanceRef="Buf-pad-led"/>
        </net>
        <net name="led">
          <portRef name="PAD" instanceRef="led_opad"/>
          <portRef name="O" instanceRef="Buf-pad-led"/>
          <portRef name="led"/>
        </net>
      </contents>
    </module>
  </library>
  <topModule name="const_zero_output" libraryRef="work_lib"/>
</design>
"#;

    let doc = Document::parse(xml).expect("xml parse");
    let design = load_fde_mapped_design_xml(doc.root_element()).expect("mapped xml import");

    let gnd = design
        .cells
        .iter()
        .find(|cell| cell.name == "GND")
        .expect("lowered gnd cell");
    assert!(gnd.is_lut());
    assert_eq!(gnd.type_name, "LUT4");
    assert_eq!(gnd.property("lut_init"), Some("0x0000"));

    let driver_net = design
        .nets
        .iter()
        .find(|net| net.name == "led")
        .expect("led net");
    assert_eq!(
        driver_net
            .driver
            .as_ref()
            .map(|driver| (driver.name.as_str(), driver.pin.as_str())),
        Some(("GND", "O"))
    );
}
