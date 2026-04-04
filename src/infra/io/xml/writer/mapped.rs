use crate::{
    domain::ConstantKind,
    ir::{Cell, Design, Endpoint, Net},
};
use std::collections::BTreeMap;

use super::is_clock_input_port;

#[derive(Default)]
struct PortHelpers {
    input_cells: Vec<Cell>,
    output_cells: Vec<Cell>,
    input_nets: Vec<Net>,
    output_nets: Vec<Net>,
}

#[derive(Default)]
struct NetOutputs {
    emitted_nets: Vec<Net>,
    output_helper_nets: Vec<Net>,
}

pub(super) fn build_fde_mapped_design(design: &Design) -> Option<Design> {
    if design.stage != "mapped" {
        return None;
    }

    let mut emitted = mapped_design_shell(design);
    let original_nets = design.nets.clone();
    let renamed_instances = renamed_instances(design);
    let (mapped_cells, constant_cells) = mapped_cells(design, &renamed_instances);
    let port_helpers = mapped_port_helpers(design);
    let net_outputs = mapped_net_outputs(design, &original_nets, &renamed_instances);

    emitted.cells.extend(mapped_cells);
    emitted.cells.extend(port_helpers.input_cells);
    emitted.cells.extend(port_helpers.output_cells);
    emitted.cells.extend(constant_cells);
    emitted.nets.extend(net_outputs.emitted_nets);
    emitted.nets.extend(port_helpers.input_nets);
    emitted.nets.extend(port_helpers.output_nets);
    emitted.nets.extend(net_outputs.output_helper_nets);

    Some(emitted)
}

fn mapped_design_shell(design: &Design) -> Design {
    Design {
        name: design.name.clone(),
        stage: design.stage.clone(),
        metadata: design.metadata.clone(),
        ports: design.ports.clone(),
        ..Design::default()
    }
}

fn renamed_instances(design: &Design) -> BTreeMap<String, String> {
    design
        .cells
        .iter()
        .filter(|cell| !cell.is_constant_source())
        .enumerate()
        .map(|(index, cell)| (cell.name.clone(), format!("id{:05}", index + 1)))
        .collect()
}

fn mapped_cells(
    design: &Design,
    renamed_instances: &BTreeMap<String, String>,
) -> (Vec<Cell>, Vec<Cell>) {
    let mut mapped_cells = Vec::new();
    let mut constant_cells = Vec::new();
    for cell in &design.cells {
        let mapped_cell = fde_mapped_cell(cell, renamed_instances);
        if cell.is_constant_source() {
            constant_cells.push(mapped_cell);
        } else {
            mapped_cells.push(mapped_cell);
        }
    }
    (mapped_cells, constant_cells)
}

fn mapped_port_helpers(design: &Design) -> PortHelpers {
    let mut helpers = PortHelpers::default();
    for port in &design.ports {
        if port.direction.is_input_like() {
            append_input_port_helpers(&mut helpers, design, &port.name);
        }
        if port.direction.is_output_like() {
            append_output_port_helpers(&mut helpers, &port.name);
        }
    }
    helpers
}

fn append_input_port_helpers(helpers: &mut PortHelpers, design: &Design, port_name: &str) {
    if is_clock_input_port(design, port_name) {
        let pad_buffer = format!("Buf-pad-{port_name}");
        let clock_buffer = format!("IBuf-clkpad-{port_name}");
        let pad_name = format!("{port_name}_ipad");
        helpers.input_cells.push(
            Cell::new(&pad_buffer, crate::domain::CellKind::Buffer, "CLKBUF")
                .with_input("I", port_name)
                .with_output("O", format!("net_Buf-pad-{port_name}")),
        );
        helpers.input_cells.push(
            Cell::new(&clock_buffer, crate::domain::CellKind::Buffer, "CLKBUF")
                .with_input("I", format!("net_Buf-pad-{port_name}"))
                .with_output("O", format!("net_IBuf-clkpad-{port_name}")),
        );
        helpers.input_cells.push(
            Cell::new(&pad_name, crate::domain::CellKind::Generic, "IPAD")
                .with_input("PAD", port_name),
        );
        helpers.input_nets.push(
            Net::new(port_name)
                .with_driver(Endpoint::port(port_name, port_name))
                .with_sink(Endpoint::cell(&pad_buffer, "I"))
                .with_sink(Endpoint::cell(&pad_name, "PAD")),
        );
        helpers.input_nets.push(
            Net::new(format!("net_Buf-pad-{port_name}"))
                .with_driver(Endpoint::cell(&pad_buffer, "O"))
                .with_sink(Endpoint::cell(&clock_buffer, "I")),
        );
    } else {
        let buffer_name = format!("Buf-pad-{port_name}");
        let pad_name = format!("{port_name}_ipad");
        helpers.input_cells.push(
            Cell::new(&buffer_name, crate::domain::CellKind::Buffer, "IBUF")
                .with_input("I", port_name)
                .with_output("O", format!("net_Buf-pad-{port_name}")),
        );
        helpers.input_cells.push(
            Cell::new(&pad_name, crate::domain::CellKind::Generic, "IPAD")
                .with_input("PAD", port_name),
        );
        helpers.input_nets.push(
            Net::new(port_name)
                .with_driver(Endpoint::port(port_name, port_name))
                .with_sink(Endpoint::cell(&buffer_name, "I"))
                .with_sink(Endpoint::cell(&pad_name, "PAD")),
        );
    }
}

fn append_output_port_helpers(helpers: &mut PortHelpers, port_name: &str) {
    let buffer_name = format!("Buf-pad-{port_name}");
    let pad_name = format!("{port_name}_opad");
    helpers.output_cells.push(
        Cell::new(&buffer_name, crate::domain::CellKind::Buffer, "OBUF")
            .with_input("I", format!("net_Buf-pad-{port_name}"))
            .with_output("O", port_name),
    );
    helpers.output_cells.push(
        Cell::new(&pad_name, crate::domain::CellKind::Generic, "OPAD")
            .with_output("PAD", port_name),
    );
}

fn mapped_net_outputs(
    design: &Design,
    original_nets: &[Net],
    renamed_instances: &BTreeMap<String, String>,
) -> NetOutputs {
    let mut outputs = NetOutputs::default();
    for net in original_nets {
        append_mapped_net_outputs(&mut outputs, net, design, renamed_instances);
    }
    outputs
}

fn append_mapped_net_outputs(
    outputs: &mut NetOutputs,
    net: &Net,
    design: &Design,
    renamed_instances: &BTreeMap<String, String>,
) {
    let mapped_driver = net
        .driver
        .as_ref()
        .map(|endpoint| fde_mapped_endpoint(endpoint, design, renamed_instances));
    let mapped_sinks = net
        .sinks
        .iter()
        .map(|endpoint| fde_mapped_endpoint(endpoint, design, renamed_instances))
        .collect::<Vec<_>>();
    let driver_port = net
        .driver
        .as_ref()
        .filter(|driver| driver.kind == crate::domain::EndpointKind::Port);
    let sink_port = net
        .sinks
        .iter()
        .find(|sink| sink.kind == crate::domain::EndpointKind::Port);

    if let Some(driver) = driver_port {
        outputs.emitted_nets.push(mapped_input_buffered_net(
            design,
            &driver.name,
            &mapped_sinks,
        ));
        return;
    }

    if let Some(port_sink) = sink_port {
        append_mapped_output_port_nets(
            outputs,
            &port_sink.name,
            &port_sink.pin,
            mapped_driver,
            &mapped_sinks,
        );
        return;
    }

    outputs.emitted_nets.push(pass_through_mapped_net(
        &net.name,
        mapped_driver,
        mapped_sinks,
    ));
}

fn mapped_input_buffered_net(design: &Design, port_name: &str, mapped_sinks: &[Endpoint]) -> Net {
    let driver_name = if is_clock_input_port(design, port_name) {
        format!("IBuf-clkpad-{port_name}")
    } else {
        format!("Buf-pad-{port_name}")
    };
    let net_name = if is_clock_input_port(design, port_name) {
        format!("net_IBuf-clkpad-{port_name}")
    } else {
        format!("net_Buf-pad-{port_name}")
    };
    mapped_net_with_sinks(net_name, Endpoint::cell(&driver_name, "O"), mapped_sinks)
}

fn append_mapped_output_port_nets(
    outputs: &mut NetOutputs,
    port_name: &str,
    port_pin: &str,
    mapped_driver: Option<Endpoint>,
    mapped_sinks: &[Endpoint],
) {
    let buffer_name = format!("Buf-pad-{port_name}");
    let pad_name = format!("{port_name}_opad");
    if let Some(driver) = mapped_driver {
        let internal_sinks = mapped_sinks
            .iter()
            .filter(|sink| !sink.is_port())
            .cloned()
            .collect::<Vec<_>>();
        let mut emitted_net =
            mapped_net_with_sinks(format!("net_Buf-pad-{port_name}"), driver, &internal_sinks);
        emitted_net = emitted_net.with_sink(Endpoint::cell(&buffer_name, "I"));
        outputs.output_helper_nets.push(emitted_net);
    }
    outputs.output_helper_nets.push(
        Net::new(port_name)
            .with_driver(Endpoint::cell(&pad_name, "PAD"))
            .with_sink(Endpoint::cell(&buffer_name, "O"))
            .with_sink(Endpoint::port(port_name, port_pin)),
    );
}

fn pass_through_mapped_net(
    net_name: &str,
    mapped_driver: Option<Endpoint>,
    mapped_sinks: Vec<Endpoint>,
) -> Net {
    let mut emitted_net = Net::new(net_name);
    emitted_net.driver = mapped_driver;
    emitted_net.sinks = mapped_sinks;
    emitted_net
}

fn mapped_net_with_sinks(net_name: impl Into<String>, driver: Endpoint, sinks: &[Endpoint]) -> Net {
    let mut emitted_net = Net::new(net_name).with_driver(driver);
    for sink in sinks {
        emitted_net = emitted_net.with_sink(sink.clone());
    }
    emitted_net
}

fn fde_mapped_cell(cell: &Cell, renamed_instances: &BTreeMap<String, String>) -> Cell {
    let mut emitted = cell.clone();
    emitted.name = renamed_instances
        .get(&cell.name)
        .cloned()
        .unwrap_or_else(|| cell.name.clone());
    emitted.cluster = None;
    match cell.constant_kind() {
        Some(ConstantKind::One) => {
            emitted.type_name = "LOGIC_1".to_string();
            for pin in &mut emitted.outputs {
                pin.port = "LOGIC_1_PIN".to_string();
            }
        }
        Some(ConstantKind::Zero) => {
            emitted.type_name = "LOGIC_0".to_string();
            for pin in &mut emitted.outputs {
                pin.port = "LOGIC_0_PIN".to_string();
            }
        }
        Some(ConstantKind::Unknown) | None => {}
    }
    emitted
}

fn fde_mapped_endpoint(
    endpoint: &Endpoint,
    design: &Design,
    renamed_instances: &BTreeMap<String, String>,
) -> Endpoint {
    if endpoint.kind != crate::domain::EndpointKind::Cell {
        return endpoint.clone();
    }
    let mut mapped = endpoint.clone();
    mapped.name = renamed_instances
        .get(&endpoint.name)
        .cloned()
        .unwrap_or_else(|| endpoint.name.clone());
    if let Some(cell) = design.cells.iter().find(|cell| cell.name == endpoint.name) {
        match cell.constant_kind() {
            Some(ConstantKind::One) => mapped.pin = "LOGIC_1_PIN".to_string(),
            Some(ConstantKind::Zero) => mapped.pin = "LOGIC_0_PIN".to_string(),
            Some(ConstantKind::Unknown) | None => {}
        }
    }
    mapped
}

#[cfg(test)]
mod tests {
    use super::build_fde_mapped_design;
    use crate::{
        domain::CellKind,
        ir::{Cell, Design, Endpoint, Net, Port},
    };

    #[test]
    fn output_port_feedback_preserves_internal_sinks_on_buffered_net() {
        let design = Design {
            name: "blinky".to_string(),
            stage: "mapped".to_string(),
            ports: vec![Port::output("led")],
            cells: vec![
                Cell::ff("ff0", "EDFFHQ")
                    .with_input("D", "next_led")
                    .with_output("Q", "led"),
                Cell::new("lut0", CellKind::Lut, "LUT2")
                    .with_input("ADR0", "led")
                    .with_input("ADR1", "led")
                    .with_output("O", "next_led"),
            ],
            nets: vec![
                Net::new("led")
                    .with_driver(Endpoint::cell("ff0", "Q"))
                    .with_sink(Endpoint::cell("lut0", "ADR0"))
                    .with_sink(Endpoint::cell("lut0", "ADR1"))
                    .with_sink(Endpoint::port("led", "led")),
                Net::new("next_led")
                    .with_driver(Endpoint::cell("lut0", "O"))
                    .with_sink(Endpoint::cell("ff0", "D")),
            ],
            ..Design::default()
        };

        let mapped = build_fde_mapped_design(&design).expect("mapped design");
        let feedback = mapped
            .nets
            .iter()
            .find(|net| net.name == "net_Buf-pad-led")
            .expect("buffered output net");

        assert_eq!(
            feedback.driver.as_ref().map(|driver| driver.name.as_str()),
            Some("id00001")
        );
        assert!(
            feedback
                .sinks
                .iter()
                .any(|sink| sink.name == "Buf-pad-led" && sink.pin == "I")
        );
        assert!(
            feedback
                .sinks
                .iter()
                .any(|sink| sink.name == "id00002" && sink.pin == "ADR0")
        );
        assert!(
            feedback
                .sinks
                .iter()
                .any(|sink| sink.name == "id00002" && sink.pin == "ADR1")
        );
    }
}
