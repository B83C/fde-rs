use crate::{
    domain::ConstantKind,
    ir::{Cell, Design, Endpoint, Net},
};
use std::collections::BTreeMap;

use super::is_clock_input_port;

pub(super) fn build_fde_mapped_design(design: &Design) -> Option<Design> {
    if design.stage != "mapped" {
        return None;
    }

    let mut emitted = Design {
        name: design.name.clone(),
        stage: design.stage.clone(),
        metadata: design.metadata.clone(),
        ports: design.ports.clone(),
        ..Design::default()
    };
    let original_nets = design.nets.clone();
    let renamed_instances = design
        .cells
        .iter()
        .filter(|cell| !cell.is_constant_source())
        .enumerate()
        .map(|(index, cell)| (cell.name.clone(), format!("id{:05}", index + 1)))
        .collect::<BTreeMap<_, _>>();

    let mut constant_cells = Vec::new();
    for cell in &design.cells {
        let mapped_cell = fde_mapped_cell(cell, &renamed_instances);
        if cell.is_constant_source() {
            constant_cells.push(mapped_cell);
        } else {
            emitted.cells.push(mapped_cell);
        }
    }

    let mut input_helper_cells = Vec::new();
    let mut output_helper_cells = Vec::new();
    let mut input_helper_nets = Vec::new();
    let mut output_helper_nets = Vec::new();
    for port in &design.ports {
        if port.direction.is_input_like() {
            if is_clock_input_port(design, &port.name) {
                let pad_buffer = format!("Buf-pad-{}", port.name);
                let clock_buffer = format!("IBuf-clkpad-{}", port.name);
                let pad_name = format!("{}_ipad", port.name);
                input_helper_cells.push(
                    Cell::new(&pad_buffer, crate::domain::CellKind::Buffer, "CLKBUF")
                        .with_input("I", &port.name)
                        .with_output("O", format!("net_Buf-pad-{}", port.name)),
                );
                input_helper_cells.push(
                    Cell::new(&clock_buffer, crate::domain::CellKind::Buffer, "CLKBUF")
                        .with_input("I", format!("net_Buf-pad-{}", port.name))
                        .with_output("O", format!("net_IBuf-clkpad-{}", port.name)),
                );
                input_helper_cells.push(
                    Cell::new(&pad_name, crate::domain::CellKind::Generic, "IPAD")
                        .with_input("PAD", &port.name),
                );

                input_helper_nets.push(
                    Net::new(&port.name)
                        .with_driver(Endpoint::port(&port.name, &port.name))
                        .with_sink(Endpoint::cell(&pad_buffer, "I"))
                        .with_sink(Endpoint::cell(&pad_name, "PAD")),
                );
                input_helper_nets.push(
                    Net::new(format!("net_Buf-pad-{}", port.name))
                        .with_driver(Endpoint::cell(&pad_buffer, "O"))
                        .with_sink(Endpoint::cell(&clock_buffer, "I")),
                );
            } else {
                let buffer_name = format!("Buf-pad-{}", port.name);
                let pad_name = format!("{}_ipad", port.name);
                input_helper_cells.push(
                    Cell::new(&buffer_name, crate::domain::CellKind::Buffer, "IBUF")
                        .with_input("I", &port.name)
                        .with_output("O", format!("net_Buf-pad-{}", port.name)),
                );
                input_helper_cells.push(
                    Cell::new(&pad_name, crate::domain::CellKind::Generic, "IPAD")
                        .with_input("PAD", &port.name),
                );
                input_helper_nets.push(
                    Net::new(&port.name)
                        .with_driver(Endpoint::port(&port.name, &port.name))
                        .with_sink(Endpoint::cell(&buffer_name, "I"))
                        .with_sink(Endpoint::cell(&pad_name, "PAD")),
                );
            }
        }

        if port.direction.is_output_like() {
            let buffer_name = format!("Buf-pad-{}", port.name);
            let pad_name = format!("{}_opad", port.name);
            output_helper_cells.push(
                Cell::new(&buffer_name, crate::domain::CellKind::Buffer, "OBUF")
                    .with_input("I", format!("net_Buf-pad-{}", port.name))
                    .with_output("O", &port.name),
            );
            output_helper_cells.push(
                Cell::new(&pad_name, crate::domain::CellKind::Generic, "OPAD")
                    .with_output("PAD", &port.name),
            );
        }
    }

    for net in &original_nets {
        let mapped_driver = net
            .driver
            .as_ref()
            .map(|endpoint| fde_mapped_endpoint(endpoint, design, &renamed_instances));
        let mapped_sinks = net
            .sinks
            .iter()
            .map(|endpoint| fde_mapped_endpoint(endpoint, design, &renamed_instances))
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
            if is_clock_input_port(design, &driver.name) {
                let clock_buffer = format!("IBuf-clkpad-{}", driver.name);
                let mut emitted_net = Net::new(format!("net_IBuf-clkpad-{}", driver.name))
                    .with_driver(Endpoint::cell(&clock_buffer, "O"));
                for sink in &mapped_sinks {
                    emitted_net = emitted_net.with_sink(sink.clone());
                }
                emitted.nets.push(emitted_net);
            } else {
                let buffer_name = format!("Buf-pad-{}", driver.name);
                let mut emitted_net = Net::new(format!("net_Buf-pad-{}", driver.name))
                    .with_driver(Endpoint::cell(&buffer_name, "O"));
                for sink in &mapped_sinks {
                    emitted_net = emitted_net.with_sink(sink.clone());
                }
                emitted.nets.push(emitted_net);
            }
            continue;
        }

        if let Some(port_sink) = sink_port {
            let buffer_name = format!("Buf-pad-{}", port_sink.name);
            let pad_name = format!("{}_opad", port_sink.name);
            if let Some(driver) = mapped_driver.clone() {
                let mut emitted_net = Net::new(format!("net_Buf-pad-{}", port_sink.name))
                    .with_driver(driver)
                    .with_sink(Endpoint::cell(&buffer_name, "I"));
                for sink in &mapped_sinks {
                    if !sink.is_port() {
                        emitted_net = emitted_net.with_sink(sink.clone());
                    }
                }
                output_helper_nets.push(emitted_net);
            }
            output_helper_nets.push(
                Net::new(&port_sink.name)
                    .with_driver(Endpoint::cell(&pad_name, "PAD"))
                    .with_sink(Endpoint::cell(&buffer_name, "O"))
                    .with_sink(Endpoint::port(&port_sink.name, &port_sink.pin)),
            );
            continue;
        }

        let mut emitted_net = Net::new(&net.name);
        emitted_net.driver = mapped_driver;
        emitted_net.sinks = mapped_sinks;
        emitted.nets.push(emitted_net);
    }

    emitted.cells.extend(input_helper_cells);
    emitted.cells.extend(output_helper_cells);
    emitted.cells.extend(constant_cells);
    emitted.nets.extend(input_helper_nets);
    emitted.nets.extend(output_helper_nets);

    Some(emitted)
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
