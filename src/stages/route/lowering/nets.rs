use super::DeviceLowering;
use crate::{
    DeviceCell, DeviceEndpoint, DeviceNet, DevicePort, DeviceSinkGuide,
    domain::NetOrigin,
    domain::{SiteKind, block_ram_route_target},
    ir::{Endpoint, EndpointTarget, PortId, RouteSegment},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

impl<'a> DeviceLowering<'a> {
    pub(super) fn materialize_nets(&mut self) {
        self.materialize_input_nets();
        self.materialize_logical_nets();
        self.materialize_output_nets();
    }

    fn materialize_input_nets(&mut self) {
        for port in &self.design.ports {
            if !port.direction.is_input_like() {
                continue;
            }
            let Some(port_id) = self.index.port_id(&port.name) else {
                continue;
            };
            let Some(driver) = self.device_port(port_id).map(port_endpoint) else {
                continue;
            };
            let Some(io_cell) = self.io_cell(port_id) else {
                continue;
            };
            let pad_sink = cell_endpoint(io_cell, "PAD");
            let gclk_driver = cell_endpoint(io_cell, "GCLKOUT");
            self.device.nets.push(
                DeviceNet::new(format!("pad::{}", port.name), NetOrigin::SyntheticPadInput)
                    .with_driver(driver)
                    .with_sink(pad_sink),
            );
            if let Some(gclk_cell) = self
                .gclk_cell(port_id)
                .map(|cell| cell_endpoint(cell, "IN"))
            {
                self.device.nets.push(
                    DeviceNet::new(format!("gclk::{}", port.name), NetOrigin::SyntheticGclk)
                        .with_driver(gclk_driver)
                        .with_sink(gclk_cell),
                );
            }
        }
    }

    fn materialize_logical_nets(&mut self) {
        for net in &self.design.nets {
            let driver = net
                .driver
                .as_ref()
                .and_then(|endpoint| self.lowered_endpoint(endpoint, true));
            let sinks = net
                .sinks
                .iter()
                .filter_map(|endpoint| self.lowered_endpoint(endpoint, false))
                .collect::<Vec<_>>();
            let sink_guides = sink_guides(driver.as_ref(), &sinks, &net.route);
            let mut lowered = DeviceNet::new(net.name.clone(), NetOrigin::Logical)
                .with_guide_tiles(guide_tiles(&net.route))
                .with_sink_guides(sink_guides);
            lowered.driver = driver;
            lowered.sinks = sinks;
            self.device.nets.push(lowered);
        }
    }

    fn materialize_output_nets(&mut self) {
        for port in &self.design.ports {
            if !port.direction.is_output_like() {
                continue;
            }
            let Some(port_id) = self.index.port_id(&port.name) else {
                continue;
            };
            let Some(port_binding) = self.device_port(port_id).map(port_endpoint) else {
                continue;
            };
            let Some(io_cell) = self.io_cell(port_id).map(|cell| cell_endpoint(cell, "PAD")) else {
                continue;
            };
            self.device.nets.push(
                DeviceNet::new(format!("pad::{}", port.name), NetOrigin::SyntheticPadOutput)
                    .with_driver(io_cell)
                    .with_sink(port_binding),
            );
        }
    }

    fn lowered_endpoint(&self, endpoint: &Endpoint, is_driver: bool) -> Option<DeviceEndpoint> {
        match self.index.resolve_endpoint(endpoint) {
            EndpointTarget::Cell(cell_id) => self
                .original_cell(cell_id)
                .map(|cell| cell_endpoint(cell, &endpoint.pin)),
            EndpointTarget::Port(port_id) => {
                let port = self.device_port(port_id)?;
                if is_driver {
                    return self.lowered_driver_port_endpoint(port_id, port);
                }
                if port.direction.is_output_like()
                    && let Some(io_cell) = self.io_cell(port_id)
                {
                    return Some(cell_endpoint(io_cell, "OUT"));
                }
                Some(port_endpoint(port))
            }
            EndpointTarget::Unknown => None,
        }
    }

    fn lowered_driver_port_endpoint(
        &self,
        port_id: PortId,
        port: &DevicePort,
    ) -> Option<DeviceEndpoint> {
        if !port.direction.is_input_like() {
            return Some(port_endpoint(port));
        }
        if let Some(gclk_cell) = self.gclk_cell(port_id) {
            return Some(cell_endpoint(gclk_cell, "OUT"));
        }
        if let Some(io_cell) = self.io_cell(port_id) {
            return Some(cell_endpoint(io_cell, "IN"));
        }
        Some(port_endpoint(port))
    }
}

fn port_endpoint(port: &DevicePort) -> DeviceEndpoint {
    DeviceEndpoint::port(
        port.port_name.clone(),
        port.pin_name.clone(),
        (port.x, port.y, port.z),
    )
}

fn cell_endpoint(cell: &DeviceCell, pin: &str) -> DeviceEndpoint {
    let (x, y, z) = if cell.site_kind_class() == SiteKind::BlockRam {
        block_ram_route_target(pin)
            .map(|target| {
                (
                    cell.x.saturating_add_signed(target.row_offset),
                    cell.y,
                    cell.z,
                )
            })
            .unwrap_or((cell.x, cell.y, cell.z))
    } else {
        (cell.x, cell.y, cell.z)
    };
    DeviceEndpoint::cell(cell.cell_name.clone(), pin.to_string(), (x, y, z))
}

fn guide_tiles(route: &[RouteSegment]) -> Vec<(usize, usize)> {
    let mut tiles = Vec::new();
    for segment in route {
        append_segment_tiles(&mut tiles, segment);
    }
    tiles.dedup();
    tiles
}

fn sink_guides(
    driver: Option<&DeviceEndpoint>,
    sinks: &[DeviceEndpoint],
    route: &[RouteSegment],
) -> Vec<DeviceSinkGuide> {
    let Some(driver) = driver else {
        return Vec::new();
    };

    let adjacency = route_adjacency(route);
    let source = (driver.x, driver.y);
    sinks
        .iter()
        .filter_map(|sink| {
            trace_route_path(source, (sink.x, sink.y), &adjacency).map(|tiles| DeviceSinkGuide {
                sink: sink.clone(),
                tiles,
            })
        })
        .collect()
}

fn route_adjacency(route: &[RouteSegment]) -> BTreeMap<(usize, usize), BTreeSet<(usize, usize)>> {
    let mut adjacency = BTreeMap::<(usize, usize), BTreeSet<(usize, usize)>>::new();
    for segment in route {
        let mut segment_tiles = Vec::new();
        append_segment_tiles(&mut segment_tiles, segment);
        for tile in &segment_tiles {
            adjacency.entry(*tile).or_default();
        }
        for window in segment_tiles.windows(2) {
            if let [from, to] = window {
                adjacency.entry(*from).or_default().insert(*to);
                adjacency.entry(*to).or_default().insert(*from);
            }
        }
    }
    adjacency
}

fn trace_route_path(
    source: (usize, usize),
    target: (usize, usize),
    adjacency: &BTreeMap<(usize, usize), BTreeSet<(usize, usize)>>,
) -> Option<Vec<(usize, usize)>> {
    if source == target {
        return Some(vec![source]);
    }
    if !adjacency.contains_key(&source) || !adjacency.contains_key(&target) {
        return None;
    }

    let mut queue = VecDeque::from([source]);
    let mut seen = BTreeSet::from([source]);
    let mut parent = BTreeMap::<(usize, usize), (usize, usize)>::new();

    while let Some(current) = queue.pop_front() {
        let Some(neighbors) = adjacency.get(&current) else {
            continue;
        };
        for neighbor in neighbors {
            if !seen.insert(*neighbor) {
                continue;
            }
            parent.insert(*neighbor, current);
            if *neighbor == target {
                return Some(reconstruct_tile_path(source, target, &parent));
            }
            queue.push_back(*neighbor);
        }
    }

    None
}

fn reconstruct_tile_path(
    source: (usize, usize),
    target: (usize, usize),
    parent: &BTreeMap<(usize, usize), (usize, usize)>,
) -> Vec<(usize, usize)> {
    let mut path = vec![target];
    let mut current = target;
    while current != source {
        let Some(previous) = parent.get(&current).copied() else {
            break;
        };
        current = previous;
        path.push(current);
    }
    path.reverse();
    path
}

fn append_segment_tiles(tiles: &mut Vec<(usize, usize)>, segment: &RouteSegment) {
    let dx = match segment.x1.cmp(&segment.x0) {
        std::cmp::Ordering::Less => -1isize,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    let dy = match segment.y1.cmp(&segment.y0) {
        std::cmp::Ordering::Less => -1isize,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    let steps = segment
        .x0
        .abs_diff(segment.x1)
        .max(segment.y0.abs_diff(segment.y1));
    let mut x = segment.x0 as isize;
    let mut y = segment.y0 as isize;

    for _ in 0..=steps {
        let point = (x as usize, y as usize);
        if tiles.last().copied() != Some(point) {
            tiles.push(point);
        }
        x += dx;
        y += dy;
    }
}

#[cfg(test)]
mod tests {
    use super::{cell_endpoint, sink_guides};
    use crate::{DeviceCell, DeviceEndpoint, domain::SiteKind, ir::RouteSegment};

    fn endpoint(name: &str, pin: &str, x: usize, y: usize) -> DeviceEndpoint {
        DeviceEndpoint::cell(name.to_string(), pin.to_string(), (x, y, 0))
    }

    #[test]
    fn sink_guides_follow_branch_specific_paths() {
        let driver = endpoint("src", "O", 0, 0);
        let sinks = vec![endpoint("dst_a", "I0", 0, 2), endpoint("dst_b", "I0", 1, 1)];
        let route = vec![
            RouteSegment::new((0, 0), (0, 1)),
            RouteSegment::new((0, 1), (0, 2)),
            RouteSegment::new((0, 1), (1, 1)),
        ];

        let guides = sink_guides(Some(&driver), &sinks, &route);
        assert_eq!(guides.len(), 2);
        assert_eq!(guides[0].sink, sinks[0]);
        assert_eq!(guides[0].tiles, vec![(0, 0), (0, 1), (0, 2)]);
        assert_eq!(guides[1].sink, sinks[1]);
        assert_eq!(guides[1].tiles, vec![(0, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn block_ram_cell_endpoints_shift_to_cpp_compatible_segment_rows() {
        let ram = DeviceCell::new("ram0", crate::domain::CellKind::BlockRam, "BLOCKRAM_2").placed(
            SiteKind::BlockRam,
            "BRAM",
            "BRAM",
            "LBRAMR12C0",
            "LBRAMD",
            (12, 5, 0),
        );

        let dia0 = cell_endpoint(&ram, "DIA0");
        let clka = cell_endpoint(&ram, "CLKA");
        let do14 = cell_endpoint(&ram, "DOA14");
        let addrb11 = cell_endpoint(&ram, "ADDRB11");

        assert_eq!((dia0.x, dia0.y, dia0.z), (9, 5, 0));
        assert_eq!((clka.x, clka.y, clka.z), (10, 5, 0));
        assert_eq!((do14.x, do14.y, do14.z), (11, 5, 0));
        assert_eq!((addrb11.x, addrb11.y, addrb11.z), (10, 5, 0));
    }
}
