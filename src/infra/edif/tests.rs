use super::parse_source;

#[test]
fn parses_renamed_instance_references() {
    let design = parse_source(
        r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port a (direction INPUT))
                      (port y (direction OUTPUT)))
                    (contents
                      (instance (rename id00001 u_lut)
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef a)
                          (portRef ADR0 (instanceRef id00001))))
                      (net net1
                        (joined
                          (portRef O (instanceRef id00001))
                          (portRef y))))))))
            "#,
    )
    .expect("parse rename");

    let net0 = design
        .nets
        .iter()
        .find(|net| net.name == "net0")
        .expect("net0");
    let sink = net0.sinks.first().expect("sink");
    assert_eq!(sink.name, "u_lut");
    assert_eq!(sink.pin, "ADR0");
}

#[test]
fn resolves_renamed_external_library_cells_before_classifying_instances() {
    let design = parse_source(
        r#"
            (edif top
              (external LIB
                (cell (rename id00001 "$_DFF_P_")
                  (cellType GENERIC)
                  (view VIEW_NETLIST
                    (viewType NETLIST)
                    (interface
                      (port C (direction INPUT))
                      (port D (direction INPUT))
                      (port Q (direction OUTPUT))))))
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port clk (direction INPUT))
                      (port q (direction OUTPUT)))
                    (contents
                      (instance (rename id100 ff0)
                        (viewRef NETLIST (cellRef id00001 (libraryRef LIB))))
                      (net clk_net
                        (joined
                          (portRef clk)
                          (portRef C (instanceRef id100))))
                      (net q_net
                        (joined
                          (portRef Q (instanceRef id100))
                          (portRef q))))))))
            "#,
    )
    .expect("parse external rename");

    let cell = design
        .cells
        .iter()
        .find(|cell| cell.name == "ff0")
        .expect("ff0");
    assert_eq!(cell.type_name, "$_DFF_P_");
    assert_eq!(cell.kind.as_str(), "ff");
}

#[test]
fn parses_string_properties_and_comments() {
    let design = parse_source(
        r#"
            (edif top
              ; comment should be ignored
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface (port a (direction INPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB)))
                        (property LABEL (string "hello world"))))))))
            "#,
    )
    .expect("parse property");

    let cell = design
        .cells
        .iter()
        .find(|cell| cell.name == "u0")
        .expect("cell");
    assert_eq!(cell.property("label"), Some("hello world"));
}

#[test]
fn parses_integer_properties_on_structural_luts() {
    let design = parse_source(
        r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface (port a (direction INPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB)))
                        (property INIT (integer 10))))))))
            "#,
    )
    .expect("parse integer property");

    let cell = design
        .cells
        .iter()
        .find(|cell| cell.name == "u0")
        .expect("cell");
    assert_eq!(cell.property("init"), Some("10"));
}

#[test]
fn parses_array_ports_and_member_references() {
    let design = parse_source(
        r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port clk (direction INPUT))
                      (port (array bus_in 2) (direction INPUT))
                      (port (array bus_out 2) (direction OUTPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef (member bus_in 0))
                          (portRef ADR0 (instanceRef u0))))
                      (net net1
                        (joined
                          (portRef (member bus_in 1))
                          (portRef ADR1 (instanceRef u0))))
                      (net net2
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member bus_out 1)))))))))
            "#,
    )
    .expect("parse array ports");

    let port_names = design
        .ports
        .iter()
        .map(|port| port.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        port_names,
        vec!["clk", "bus_in[1]", "bus_in[0]", "bus_out[1]", "bus_out[0]"]
    );

    let net0 = design
        .nets
        .iter()
        .find(|net| net.name == "net0")
        .expect("net0");
    assert_eq!(
        net0.driver.as_ref().map(|driver| driver.name.as_str()),
        Some("bus_in[1]")
    );

    let net2 = design
        .nets
        .iter()
        .find(|net| net.name == "net2")
        .expect("net2");
    assert_eq!(
        net2.sinks.first().map(|sink| sink.name.as_str()),
        Some("bus_out[0]")
    );
}

#[test]
fn resolves_renamed_array_ports_using_member_ordinals() {
    let design = parse_source(
        r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port (array (rename BUS "bus[3:1]") 3) (direction OUTPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 0))))
                      (net net1
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 1))))
                      (net net2
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 2)))))))))
            "#,
    )
    .expect("parse renamed array");

    let port_names = design
        .ports
        .iter()
        .map(|port| port.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(port_names, vec!["bus[3]", "bus[2]", "bus[1]"]);

    let net0 = design
        .nets
        .iter()
        .find(|net| net.name == "net0")
        .expect("net0");
    assert_eq!(
        net0.sinks.first().map(|sink| sink.name.as_str()),
        Some("bus[3]")
    );

    let net2 = design
        .nets
        .iter()
        .find(|net| net.name == "net2")
        .expect("net2");
    assert_eq!(
        net2.sinks.first().map(|sink| sink.name.as_str()),
        Some("bus[1]")
    );
}
