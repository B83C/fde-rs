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
        design.cells.iter().any(
            |cell| cell.name == "iSlice__0__::lut1" && cell.property("lut_init") == Some("0x0")
        )
    );
    let led_net = design
        .nets
        .iter()
        .find(|net| net.name == "led")
        .expect("logical led net");
    assert_eq!(
        led_net
            .driver
            .as_ref()
            .map(|endpoint| endpoint.name.as_str()),
        Some("iSlice__0__::lut1")
    );
}
