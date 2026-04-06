use crate::{
    cil::Cil,
    constraints::{
        SharedConstraints, apply_constraints, ensure_cluster_positions, ensure_port_positions,
    },
    domain::PrimitiveKind,
    ir::{Design, RoutePip, RouteSegment},
    report::{StageOutput, StageReport},
    resource::{Arch, SharedArch},
};
use anyhow::{Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use super::{DeviceRouteImage, route_device_design};

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
    let result = run_with_artifacts(design, options)?;
    Ok(StageOutput {
        value: result.value.design,
        report: result.report,
    })
}

pub fn run_with_artifacts(
    mut design: Design,
    options: &RouteOptions,
) -> Result<StageOutput<RouteStageArtifacts>> {
    let Some(cil) = options.cil.as_ref() else {
        bail!(
            "physical routing now requires a CIL library; pass --cil or configure a resource bundle"
        )
    };

    design.stage = "routed".to_string();
    apply_constraints(&mut design, &options.arch, &options.constraints);
    ensure_port_positions(&mut design, &options.arch);
    if !design.clusters.is_empty() {
        ensure_cluster_positions(&design)?;
    }
    if design
        .cells
        .iter()
        .any(|cell| matches!(cell.primitive_kind(), PrimitiveKind::BlockRam))
    {
        bail!(
            "Block RAM cells are imported, packed, and placed, but BRAM macro routing/bitgen is not implemented yet."
        );
    }

    let Some(device_design) = options.device_design.clone() else {
        bail!("route stage requires a prepared device design")
    };
    let route_image = route_device_design(&device_design, &options.arch, &options.arch_path, cil)?;
    let programmed_sites = route_image
        .pips
        .iter()
        .map(|pip| (pip.tile_name.as_str(), pip.site_name.as_str()))
        .collect::<BTreeSet<_>>()
        .len();
    let device_net_count = device_design.nets.len();
    apply_route_image(&mut design, &route_image, &options.arch);

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
        cil::Cil,
        domain::{CellKind, ClusterKind},
        ir::{Cell, Cluster, Design, Endpoint, Net, RoutePip, RouteSegment},
        resource::Arch,
    };

    use super::{DeviceRouteImage, apply_route_image, derive_segments_from_pips};

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
    fn route_stage_rejects_block_ram_until_macro_routing_exists() {
        let design = Design {
            cells: vec![
                Cell::new("ram0", CellKind::BlockRam, "BLOCKRAM_SINGLE_PORT")
                    .with_input("CLK", "clk")
                    .with_output("DO0", "q")
                    .in_cluster("bram0"),
            ],
            nets: vec![
                Net::new("clk")
                    .with_driver(Endpoint::port("clk", "IN"))
                    .with_sink(Endpoint::cell("ram0", "CLK")),
            ],
            clusters: vec![
                Cluster::new("bram0", ClusterKind::BlockRam)
                    .with_member("ram0")
                    .with_capacity(1)
                    .fixed_at_slot(1, 0, 0),
            ],
            ..Design::default()
        };

        let err = super::run(
            design,
            &super::RouteOptions {
                arch: Arch {
                    name: "mini".to_string(),
                    width: 4,
                    height: 4,
                    ..Arch::default()
                }
                .into(),
                arch_path: std::path::PathBuf::new(),
                constraints: Vec::new().into(),
                cil: Some(Cil::default()),
                device_design: None,
            },
        )
        .expect_err("block RAM should fail before routing");
        assert!(err.to_string().contains("Block RAM cells"));
    }
}
