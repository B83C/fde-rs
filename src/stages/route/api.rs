use crate::{
    cil::Cil,
    constraints::{
        SharedConstraints, apply_constraints, ensure_cluster_positions, ensure_port_positions,
    },
    ir::{Design, RoutePip, RouteSegment},
    report::{StageOutput, StageReport, StageReporter, emit_stage_info},
    resource::{Arch, SharedArch},
};
use anyhow::{Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use super::{DeviceRouteImage, route_device_design, route_device_design_with_reporter};

#[derive(Debug, Clone)]
pub struct RouteStageArtifacts {
    pub design: Design,
    pub device_design: crate::DeviceDesign,
    pub route_image: DeviceRouteImage,
}

#[derive(Debug, Clone)]
pub struct RouteOptions {
    pub arch: SharedArch,
    pub arch_path: PathBuf,
    pub constraints: SharedConstraints,
    pub cil: Option<Cil>,
    pub device_design: Option<crate::DeviceDesign>,
}

pub fn run(design: Design, options: &RouteOptions) -> Result<StageOutput<Design>> {
    let result = run_with_artifacts_internal(design, options, None)?;
    Ok(StageOutput {
        value: result.value.design,
        report: result.report,
    })
}

pub fn run_with_reporter(
    design: Design,
    options: &RouteOptions,
    reporter: &mut dyn StageReporter,
) -> Result<StageOutput<Design>> {
    let result = run_with_artifacts_internal(design, options, Some(reporter))?;
    Ok(StageOutput {
        value: result.value.design,
        report: result.report,
    })
}

pub fn run_with_artifacts(
    design: Design,
    options: &RouteOptions,
) -> Result<StageOutput<RouteStageArtifacts>> {
    run_with_artifacts_internal(design, options, None)
}

pub fn run_with_artifacts_and_reporter(
    design: Design,
    options: &RouteOptions,
    reporter: &mut dyn StageReporter,
) -> Result<StageOutput<RouteStageArtifacts>> {
    run_with_artifacts_internal(design, options, Some(reporter))
}

fn run_with_artifacts_internal(
    mut design: Design,
    options: &RouteOptions,
    mut reporter: Option<&mut dyn StageReporter>,
) -> Result<StageOutput<RouteStageArtifacts>> {
    let Some(cil) = options.cil.as_ref() else {
        bail!(
            "physical routing now requires a CIL library; pass --cil or configure a resource bundle"
        )
    };

    design.stage = "routed".to_string();
    emit_stage_info(
        &mut reporter,
        "route",
        format!(
            "preparing route stage for {} logical nets",
            design.nets.len()
        ),
    );
    apply_constraints(&mut design, &options.arch, &options.constraints);
    ensure_port_positions(&mut design, &options.arch);
    if !design.clusters.is_empty() {
        ensure_cluster_positions(&design)?;
    }

    let Some(device_design) = options.device_design.clone() else {
        bail!("route stage requires a prepared device design")
    };
    emit_stage_info(
        &mut reporter,
        "route",
        format!(
            "routing {} device nets on architecture '{}'",
            device_design.nets.len(),
            options.arch.name
        ),
    );
    let route_image = match reporter.as_deref_mut() {
        Some(reporter) => route_device_design_with_reporter(
            &device_design,
            &options.arch,
            &options.arch_path,
            cil,
            reporter,
        )?,
        None => route_device_design(&device_design, &options.arch, &options.arch_path, cil)?,
    };
    let programmed_sites = route_image
        .pips
        .iter()
        .map(|pip| (pip.tile_name.as_str(), pip.site_name.as_str()))
        .collect::<BTreeSet<_>>()
        .len();
    let device_net_count = device_design.nets.len();
    apply_route_image(&mut design, &route_image, &options.arch);
    emit_stage_info(
        &mut reporter,
        "route",
        format!(
            "route stage materialized {} physical pips across {} sites",
            route_image.pips.len(),
            programmed_sites
        ),
    );

    let mut report = StageReport::new("route");
    report.metric("physical_pip_count", route_image.pips.len());
    report.metric("routed_site_count", programmed_sites);
    report.metric("device_net_count", device_net_count);
    report.push(format!(
        "Materialized {} physical pips across {} routed sites for {} device nets.",
        route_image.pips.len(),
        programmed_sites,
        device_net_count,
    ));
    for note in &route_image.notes {
        if is_route_warning(note) {
            report.warn(note.clone());
        } else {
            report.push(note.clone());
        }
    }

    Ok(StageOutput {
        value: RouteStageArtifacts {
            design,
            device_design,
            route_image,
        },
        report,
    })
}

fn is_route_warning(note: &str) -> bool {
    let lowered = note.to_ascii_lowercase();
    lowered.contains("could not find a rust route")
        || lowered.contains("has no routed driver")
        || lowered.contains("not a routable cell")
        || lowered.contains("has no route-source mapping")
}

fn apply_route_image(design: &mut Design, route_image: &DeviceRouteImage, arch: &Arch) {
    let mut by_net = BTreeMap::<String, Vec<RoutePip>>::new();
    for pip in &route_image.pips {
        by_net
            .entry(logical_route_net_name(&pip.net_name).to_string())
            .or_default()
            .push(RoutePip::new(
                (pip.x, pip.y),
                pip.from_net.clone(),
                pip.to_net.clone(),
            ));
    }

    for net in &mut design.nets {
        net.route_pips = by_net.remove(net.name.as_str()).unwrap_or_default();
        net.route = derive_segments_from_pips(&net.route_pips);
        net.estimated_delay_ns = if net.route.is_empty() {
            estimate_pip_delay(&net.route_pips, arch)
        } else {
            estimate_segment_delay(&net.route, arch)
        };
    }
}

fn logical_route_net_name(device_net_name: &str) -> &str {
    device_net_name
        .strip_prefix("gclk::")
        .unwrap_or(device_net_name)
}

fn estimate_pip_delay(route_pips: &[RoutePip], arch: &Arch) -> f64 {
    route_pips.len() as f64 * (arch.wire_r + arch.wire_c + 0.02)
}

fn estimate_segment_delay(route: &[RouteSegment], arch: &Arch) -> f64 {
    let length = route.iter().map(RouteSegment::length).sum::<usize>() as f64;
    let bends = route
        .windows(2)
        .filter(|window| match window {
            [lhs, rhs] => (lhs.x0 == lhs.x1) != (rhs.x0 == rhs.x1),
            _ => false,
        })
        .count() as f64;
    length * (arch.wire_r + arch.wire_c + 0.02) + bends * 0.05
}

fn derive_segments_from_pips(pips: &[RoutePip]) -> Vec<RouteSegment> {
    let mut positions = Vec::<(usize, usize)>::new();
    for pip in pips {
        let position = (pip.x, pip.y);
        if positions.last().copied() != Some(position) {
            positions.push(position);
        }
    }
    match positions.as_slice() {
        [] => Vec::new(),
        [single] => vec![RouteSegment::new(*single, *single)],
        _ => positions
            .windows(2)
            .filter_map(|window| match window {
                [start, end] => Some(RouteSegment::new(*start, *end)),
                _ => None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cil::load_cil,
        constraints::load_constraints,
        domain::{CellKind, ClusterKind},
        ir::{Cell, Cluster, Design, Endpoint, Net, Port, RoutePip, RouteSegment},
        resource::{ResourceBundle, load_arch},
    };
    use std::path::PathBuf;

    use super::{DeviceRouteImage, RouteOptions, apply_route_image, derive_segments_from_pips};

    #[test]
    fn apply_route_image_merges_synthetic_gclk_pips_into_logical_clock_net() {
        let mut design = Design {
            nets: vec![
                Net::new("clk")
                    .with_driver(Endpoint::port("clk", "clk"))
                    .with_sink(Endpoint::cell("u_ff", "CK")),
            ],
            ..Design::default()
        };
        let route_image = DeviceRouteImage {
            pips: vec![
                crate::route::DeviceRoutePip {
                    net_name: "gclk::clk".to_string(),
                    tile_name: "BM".to_string(),
                    tile_type: "CLKB".to_string(),
                    site_name: "GCLKBUF1".to_string(),
                    site_type: "GCLK".to_string(),
                    x: 34,
                    y: 27,
                    from_net: "CLKB_CLKPAD1".to_string(),
                    to_net: "CLKB_GCLKBUF1_IN".to_string(),
                    bits: Vec::new(),
                },
                crate::route::DeviceRoutePip {
                    net_name: "clk".to_string(),
                    tile_name: "BM".to_string(),
                    tile_type: "CLKB".to_string(),
                    site_name: "GSB_CLKB".to_string(),
                    site_type: "GSB".to_string(),
                    x: 34,
                    y: 27,
                    from_net: "CLKB_GCLK1_PW".to_string(),
                    to_net: "CLKB_GCLK1".to_string(),
                    bits: Vec::new(),
                },
            ],
            ..DeviceRouteImage::default()
        };

        apply_route_image(&mut design, &route_image, &crate::resource::Arch::default());

        assert_eq!(
            design.nets[0].route_pips,
            vec![
                RoutePip::new((34, 27), "CLKB_CLKPAD1", "CLKB_GCLKBUF1_IN"),
                RoutePip::new((34, 27), "CLKB_GCLK1_PW", "CLKB_GCLK1"),
            ]
        );
        assert_eq!(
            design.nets[0].route,
            vec![RouteSegment::new((34, 27), (34, 27))]
        );
    }

    #[test]
    fn derive_segments_from_pips_collapses_duplicate_positions() {
        let segments = derive_segments_from_pips(&[
            RoutePip::new((1, 2), "a", "b"),
            RoutePip::new((1, 2), "b", "c"),
            RoutePip::new((2, 2), "c", "d"),
            RoutePip::new((2, 3), "d", "e"),
        ]);

        assert_eq!(
            segments,
            vec![
                RouteSegment::new((1, 2), (2, 2)),
                RouteSegment::new((2, 2), (2, 3)),
            ]
        );
    }

    #[test]
    fn route_stage_routes_single_port_block_ram_data_pins() -> anyhow::Result<()> {
        let Some(bundle) =
            ResourceBundle::discover_from(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))).ok()
        else {
            return Ok(());
        };
        let arch_path = bundle.root.join("fdp3p7_arch.xml");
        let cil_path = bundle.root.join("fdp3p7_cil.xml");
        if !arch_path.exists() || !cil_path.exists() {
            return Ok(());
        }

        let arch = load_arch(&arch_path)?;
        let cil = load_cil(&cil_path)?;

        let mut gnd = Cell::new("GND", CellKind::Lut, "LUT4")
            .with_output("O", "GND_NET")
            .in_cluster("clb_0001");
        gnd.set_property("lut_init", "0x0000");

        let mut ram = Cell::new("ram", CellKind::BlockRam, "BLOCKRAM_1")
            .with_input("ADDR0", "GND_NET")
            .with_input("ADDR1", "GND_NET")
            .with_input("ADDR2", "GND_NET")
            .with_input("ADDR3", "GND_NET")
            .with_input("ADDR4", "GND_NET")
            .with_input("ADDR5", "GND_NET")
            .with_input("ADDR6", "GND_NET")
            .with_input("ADDR7", "GND_NET")
            .with_input("ADDR8", "GND_NET")
            .with_input("ADDR9", "GND_NET")
            .with_input("ADDR10", "GND_NET")
            .with_input("ADDR11", "GND_NET")
            .with_input("CLK", "GND_NET")
            .with_input("DI", "GND_NET")
            .with_input("EN", "GND_NET")
            .with_input("RST", "GND_NET")
            .with_input("WE", "GND_NET")
            .with_output("DO", "q")
            .in_cluster("bram_0000");
        ram.set_property("PORT_ATTR", "4096X1");

        let gnd_net = [
            "ADDR0", "ADDR1", "ADDR2", "ADDR3", "ADDR4", "ADDR5", "ADDR6", "ADDR7", "ADDR8",
            "ADDR9", "ADDR10", "ADDR11", "CLK", "DI", "EN", "RST", "WE",
        ]
        .into_iter()
        .fold(
            Net::new("GND_NET").with_driver(Endpoint::cell("GND", "O")),
            |net, pin| net.with_sink(Endpoint::cell("ram", pin)),
        );

        let design = Design {
            name: "bram-route".to_string(),
            stage: "placed".to_string(),
            ports: vec![Port::output("q")],
            cells: vec![gnd, ram],
            nets: vec![
                gnd_net,
                Net::new("q")
                    .with_driver(Endpoint::cell("ram", "DO"))
                    .with_sink(Endpoint::port("q", "q")),
            ],
            clusters: vec![
                Cluster::new("bram_0000", ClusterKind::BlockRam)
                    .with_member("ram")
                    .with_capacity(1)
                    .fixed_at_slot(4, 0, 0),
                Cluster::logic("clb_0001")
                    .with_member("GND")
                    .with_capacity(4)
                    .fixed_at_slot(4, 2, 0),
            ],
            ..Design::default()
        };

        let device_design = crate::route::lower_design(design.clone(), &arch, Some(&cil), &[])?;
        let result = super::run_with_artifacts(
            design,
            &RouteOptions {
                arch: std::sync::Arc::new(arch),
                arch_path,
                constraints: Vec::new().into(),
                cil: Some(cil),
                device_design: Some(device_design),
            },
        )?;

        assert!(
            result
                .value
                .design
                .nets
                .iter()
                .all(|net| !net.route_pips.is_empty()),
            "every logical BRAM net should gain physical route pips"
        );
        assert!(
            !result
                .report
                .warnings
                .iter()
                .any(|warning| warning.contains("route-source mapping")
                    || warning.contains("route-sink mapping")
                    || warning.contains("could not find a Rust route")),
            "single-port BRAM data/control pins should route without BRAM-specific warnings"
        );
        Ok(())
    }

    #[test]
    fn route_stage_uses_dedicated_clock_sink_path_for_block_ram_clock() -> anyhow::Result<()> {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let Some(bundle) = ResourceBundle::discover_from(&repo_root).ok() else {
            return Ok(());
        };
        let arch_path = bundle.root.join("fdp3p7_arch.xml");
        let cil_path = bundle.root.join("fdp3p7_cil.xml");
        let constraints_path = repo_root.join("tests/fixtures/fdp3p7-constraints.xml");
        if !arch_path.exists() || !cil_path.exists() || !constraints_path.exists() {
            return Ok(());
        }

        let arch = load_arch(&arch_path)?;
        let cil = load_cil(&cil_path)?;
        let constraints = load_constraints(&constraints_path)?;

        let mut gnd = Cell::new("GND", CellKind::Lut, "LUT4")
            .with_output("O", "GND_NET")
            .in_cluster("clb_gnd");
        gnd.set_property("lut_init", "0x0000");

        let mut ram = Cell::new("ram", CellKind::BlockRam, "BLOCKRAM_1")
            .with_input("ADDR0", "GND_NET")
            .with_input("ADDR1", "GND_NET")
            .with_input("ADDR2", "GND_NET")
            .with_input("ADDR3", "GND_NET")
            .with_input("ADDR4", "GND_NET")
            .with_input("ADDR5", "GND_NET")
            .with_input("ADDR6", "GND_NET")
            .with_input("ADDR7", "GND_NET")
            .with_input("ADDR8", "GND_NET")
            .with_input("ADDR9", "GND_NET")
            .with_input("ADDR10", "GND_NET")
            .with_input("ADDR11", "GND_NET")
            .with_input("CLK", "clk")
            .with_input("DI", "GND_NET")
            .with_input("EN", "GND_NET")
            .with_input("RST", "GND_NET")
            .with_input("WE", "GND_NET")
            .in_cluster("bram0");
        ram.set_property("PORT_ATTR", "4096X1");

        let ff = Cell::ff("ff0", "DFFHQ")
            .with_input("D", "GND_NET")
            .with_input("CLK", "clk")
            .in_cluster("clb_ff");

        let gnd_net = [
            Endpoint::cell("ram", "ADDR0"),
            Endpoint::cell("ram", "ADDR1"),
            Endpoint::cell("ram", "ADDR2"),
            Endpoint::cell("ram", "ADDR3"),
            Endpoint::cell("ram", "ADDR4"),
            Endpoint::cell("ram", "ADDR5"),
            Endpoint::cell("ram", "ADDR6"),
            Endpoint::cell("ram", "ADDR7"),
            Endpoint::cell("ram", "ADDR8"),
            Endpoint::cell("ram", "ADDR9"),
            Endpoint::cell("ram", "ADDR10"),
            Endpoint::cell("ram", "ADDR11"),
            Endpoint::cell("ram", "DI"),
            Endpoint::cell("ram", "EN"),
            Endpoint::cell("ram", "RST"),
            Endpoint::cell("ram", "WE"),
            Endpoint::cell("ff0", "D"),
        ]
        .into_iter()
        .fold(
            Net::new("GND_NET").with_driver(Endpoint::cell("GND", "O")),
            |net, sink| net.with_sink(sink),
        );

        let design = Design {
            name: "bram-clock-route".to_string(),
            stage: "placed".to_string(),
            ports: vec![Port::input("clk")],
            cells: vec![gnd, ram, ff],
            nets: vec![
                gnd_net,
                Net::new("clk")
                    .with_driver(Endpoint::port("clk", "IN"))
                    .with_sink(Endpoint::cell("ram", "CLK"))
                    .with_sink(Endpoint::cell("ff0", "CLK")),
            ],
            clusters: vec![
                Cluster::new("bram0", ClusterKind::BlockRam)
                    .with_member("ram")
                    .with_capacity(1)
                    .fixed_at_slot(4, 0, 0),
                Cluster::logic("clb_gnd")
                    .with_member("GND")
                    .with_capacity(4)
                    .fixed_at_slot(4, 2, 0),
                Cluster::logic("clb_ff")
                    .with_member("ff0")
                    .with_capacity(4)
                    .fixed_at_slot(5, 2, 0),
            ],
            ..Design::default()
        };

        let device_design =
            crate::route::lower_design(design.clone(), &arch, Some(&cil), &constraints)?;
        let result = super::run_with_artifacts(
            design,
            &RouteOptions {
                arch: std::sync::Arc::new(arch),
                arch_path,
                constraints: constraints.into(),
                cil: Some(cil),
                device_design: Some(device_design),
            },
        )?;

        let clock_pips = result
            .value
            .route_image
            .pips
            .iter()
            .filter(|pip| pip.net_name == "clk")
            .collect::<Vec<_>>();

        assert!(
            clock_pips
                .iter()
                .any(|pip| pip.to_net == "BRAM_CLKA" && pip.from_net.starts_with("BRAM_GCLKIN")),
            "expected dedicated BRAM clock sink arc on logical clk net, got {:?}",
            clock_pips
                .iter()
                .filter(|pip| pip.to_net == "BRAM_CLKA" || pip.from_net.starts_with("BRAM_"))
                .map(|pip| (pip.x, pip.y, pip.from_net.as_str(), pip.to_net.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(
            !clock_pips.iter().any(|pip| {
                pip.to_net == "BRAM_CLKA" && !pip.from_net.starts_with("BRAM_GCLKIN")
            }),
            "BRAM clock sink should not be reached from generic local wires"
        );

        Ok(())
    }
}
