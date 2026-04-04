use crate::{
    bitgen::{BitgenOptions, run as run_bitgen},
    cil::load_cil,
    constraints::load_constraints,
    domain::{
        NetOrigin, SiteKind, is_clock_distribution_wire_name, is_clock_sink_wire_name,
        is_directional_channel_wire_name, is_hex_like_wire_name, is_long_wire_name,
        is_pad_stub_wire_name,
    },
    ir::{Cell, Cluster, Design, Endpoint, Net, RoutePip, RouteSegment},
    map::{MapOptions, load_input, run as run_map},
    pack::{PackOptions, run as run_pack},
    place::{PlaceMode, PlaceOptions, run as run_place},
    resource::{TileKind, load_arch, load_delay_model},
    route::{DeviceRouteImage, lower_design, route_device_design},
};
use anyhow::Result;
use std::{collections::BTreeSet, path::PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn segments_from_points(points: &[(usize, usize)]) -> Vec<RouteSegment> {
    points
        .windows(2)
        .map(|window| RouteSegment::new(window[0], window[1]))
        .collect()
}

fn with_route_pips(mut design: Design, route_image: &DeviceRouteImage) -> Design {
    for net in &mut design.nets {
        net.route.clear();
        net.route_pips = route_image
            .pips
            .iter()
            .filter(|pip| pip.net_name == net.name)
            .map(|pip| RoutePip::new((pip.x, pip.y), pip.from_net.clone(), pip.to_net.clone()))
            .collect();
    }
    design
}

fn guided_logic_design(
    src: (usize, usize),
    dst: (usize, usize),
    guide_points: &[(usize, usize)],
) -> Design {
    Design {
        name: "guided-device-route".to_string(),
        stage: "routed".to_string(),
        cells: vec![
            Cell::lut("src", "LUT4")
                .with_output("O", "link")
                .in_cluster("clb_src"),
            Cell::lut("dst", "LUT4")
                .with_input("ADR0", "link")
                .in_cluster("clb_dst"),
        ],
        nets: vec![Net {
            route: segments_from_points(guide_points),
            ..Net::new("link")
                .with_driver(Endpoint::cell("src", "O"))
                .with_sink(Endpoint::cell("dst", "ADR0"))
        }],
        clusters: vec![
            Cluster::logic("clb_src")
                .with_member("src")
                .with_capacity(1)
                .at(src.0, src.1),
            Cluster::logic("clb_dst")
                .with_member("dst")
                .with_capacity(1)
                .at(dst.0, dst.1),
        ],
        ..Design::default()
    }
}

type GuidedPair = (
    (usize, usize),
    (usize, usize),
    Vec<(usize, usize)>,
    Vec<(usize, usize)>,
);

fn find_guided_pair(arch: &crate::resource::Arch) -> Option<GuidedPair> {
    let sites = arch.logic_sites();
    for &src in &sites {
        for &dst in &sites {
            if src == dst || src.0 != dst.0 || src.1.abs_diff(dst.1) < 4 {
                continue;
            }
            let (low_y, high_y) = if src.1 < dst.1 {
                (src.1, dst.1)
            } else {
                (dst.1, src.1)
            };
            for detour_x in [src.0.saturating_add(1), src.0.saturating_sub(1)] {
                if detour_x == src.0 || detour_x >= arch.width {
                    continue;
                }
                let direct = (low_y..=high_y).map(|y| (src.0, y)).collect::<Vec<_>>();
                let mut detour = Vec::new();
                detour.push((src.0, low_y));
                detour.push((detour_x, low_y));
                detour.extend((low_y + 1..=high_y).map(|y| (detour_x, y)));
                detour.push((src.0, high_y));
                if direct
                    .iter()
                    .chain(detour.iter())
                    .all(|&(x, y)| arch.tile_at(x, y).is_some())
                {
                    let (ordered_src, ordered_dst) = if src.1 <= dst.1 {
                        (src, dst)
                    } else {
                        (dst, src)
                    };
                    return Some((ordered_src, ordered_dst, direct, detour));
                }
            }
        }
    }
    None
}

fn find_order_sensitive_pair(arch: &crate::resource::Arch) -> Option<GuidedPair> {
    let sites = arch.logic_sites();
    for &src in &sites {
        for &dst in &sites {
            if src == dst || src.0 != dst.0 || src.1.abs_diff(dst.1) != 2 {
                continue;
            }
            let (ordered_src, ordered_dst) = if src.1 <= dst.1 {
                (src, dst)
            } else {
                (dst, src)
            };
            let direct = vec![ordered_src, (ordered_src.0, ordered_src.1 + 1), ordered_dst];
            for detour_x in [
                ordered_src.0.saturating_add(1),
                ordered_src.0.saturating_sub(1),
            ] {
                if detour_x == ordered_src.0 || detour_x >= arch.width {
                    continue;
                }
                let detour = vec![
                    ordered_src,
                    (ordered_src.0, ordered_src.1 + 1),
                    (detour_x, ordered_src.1 + 1),
                    (detour_x, ordered_dst.1),
                    ordered_dst,
                ];
                if direct
                    .iter()
                    .chain(detour.iter())
                    .all(|&(x, y)| arch.tile_at(x, y).is_some())
                {
                    return Some((ordered_src, ordered_dst, direct, detour));
                }
            }
        }
    }
    None
}

#[test]
fn lowering_materializes_clock_and_io_sites_when_external_resources_are_available() -> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    let delay_path = bundle.root.join("fdp3p7_dly.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let design = load_input(&repo_root().join("tests/fixtures/blinky-yosys.edf"))?;
    let mapped = run_map(design, &MapOptions::default())?.value.design;
    let packed = run_pack(
        mapped,
        &PackOptions {
            family: Some("fdp3".to_string()),
            ..PackOptions::default()
        },
    )?
    .value;
    let arch = load_arch(&arch_path)?;
    let delay = load_delay_model(Some(&delay_path))?;
    let constraints = load_constraints(&repo_root().join("tests/fixtures/fdp3p7-constraints.xml"))?;
    let placed = run_place(
        packed,
        &PlaceOptions {
            arch: arch.clone().into(),
            delay: delay.map(Into::into),
            constraints: constraints.clone().into(),
            mode: PlaceMode::TimingDriven,
            seed: 0xFDE_2024,
        },
    )?
    .value;
    let cil = load_cil(&cil_path)?;
    let lowered = lower_design(placed, &arch, Some(&cil), &constraints)?;

    assert!(lowered.ports.iter().any(|port| {
        port.port_name == "clk"
            && port.site_kind == SiteKind::GclkIob
            && port.tile_kind() == TileKind::ClockBottom
            && port.site_name == "GCLKIOB0"
    }));
    assert!(lowered.cells.iter().any(|cell| {
        cell.synthetic
            && cell.type_name == "GCLK"
            && cell.cell_name == "$gclk$clk"
            && cell.site_name == "GCLKBUF0"
    }));
    assert!(lowered.cells.iter().any(|cell| !cell.synthetic
        && cell.type_name == "LUT2"
        && cell.site_kind == SiteKind::LogicSlice));
    assert!(
        lowered
            .nets
            .iter()
            .any(|net| net.origin == NetOrigin::SyntheticGclk)
    );

    Ok(())
}

#[test]
fn exact_clock_routing_connects_gclk_pad_into_global_buffer_when_resources_are_available()
-> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    let delay_path = bundle.root.join("fdp3p7_dly.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let design = load_input(&repo_root().join("tests/fixtures/blinky-yosys.edf"))?;
    let mapped = run_map(design, &MapOptions::default())?.value.design;
    let packed = run_pack(
        mapped,
        &PackOptions {
            family: Some("fdp3".to_string()),
            ..PackOptions::default()
        },
    )?
    .value;
    let arch = load_arch(&arch_path)?;
    let delay = load_delay_model(Some(&delay_path))?;
    let constraints = load_constraints(&repo_root().join("tests/fixtures/fdp3p7-constraints.xml"))?;
    let placed = run_place(
        packed,
        &PlaceOptions {
            arch: arch.clone().into(),
            delay: delay.map(Into::into),
            constraints: constraints.clone().into(),
            mode: PlaceMode::TimingDriven,
            seed: 0xFDE_2024,
        },
    )?
    .value;
    let cil = load_cil(&cil_path)?;
    let lowered = lower_design(placed, &arch, Some(&cil), &constraints)?;

    let gclk_pad = lowered
        .cells
        .iter()
        .find(|cell| {
            cell.synthetic && cell.site_kind == SiteKind::GclkIob && cell.cell_name == "$iob$clk"
        })
        .expect("synthetic clock pad cell");
    let gclk = lowered
        .cells
        .iter()
        .find(|cell| {
            cell.synthetic && cell.site_kind == SiteKind::Gclk && cell.cell_name == "$gclk$clk"
        })
        .expect("synthetic global clock buffer");
    let expected_from = format!(
        "{}_CLKPAD{}",
        gclk_pad.tile_wire_prefix(),
        gclk_pad.site_slot()
    );
    let expected_to = format!("{}_GCLKBUF{}_IN", gclk.tile_wire_prefix(), gclk.z);

    let route_image = crate::route::route_device_design(&lowered, &arch, &arch_path, &cil)?;

    assert!(route_image.pips.iter().any(|pip| {
        pip.net_name == "gclk::clk" && pip.from_net == expected_from && pip.to_net == expected_to
    }));

    Ok(())
}

#[test]
fn exact_logical_clock_routing_uses_cpp_compatible_clock_branch_wires_when_needed() -> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    let delay_path = bundle.root.join("fdp3p7_dly.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let design = load_input(&repo_root().join("tests/fixtures/blinky-yosys.edf"))?;
    let mapped = run_map(design, &MapOptions::default())?.value.design;
    let packed = run_pack(
        mapped,
        &PackOptions {
            family: Some("fdp3".to_string()),
            ..PackOptions::default()
        },
    )?
    .value;
    let arch = load_arch(&arch_path)?;
    let delay = load_delay_model(Some(&delay_path))?;
    let constraints = load_constraints(&repo_root().join("tests/fixtures/fdp3p7-constraints.xml"))?;
    let placed = run_place(
        packed,
        &PlaceOptions {
            arch: arch.clone().into(),
            delay: delay.map(Into::into),
            constraints: constraints.clone().into(),
            mode: PlaceMode::TimingDriven,
            seed: 0xFDE_2024,
        },
    )?
    .value;
    let cil = load_cil(&cil_path)?;
    let lowered = lower_design(placed, &arch, Some(&cil), &constraints)?;

    let gclk = lowered
        .cells
        .iter()
        .find(|cell| {
            cell.synthetic && cell.site_kind == SiteKind::Gclk && cell.cell_name == "$gclk$clk"
        })
        .expect("synthetic global clock buffer");
    let logical_clock_net = lowered
        .nets
        .iter()
        .find(|net| {
            net.driver.as_ref().is_some_and(|driver| {
                driver.kind == crate::domain::EndpointKind::Cell
                    && driver.name == gclk.cell_name
                    && driver.pin == "OUT"
            })
        })
        .expect("logical clock net");
    let expected_from = format!("{}_GCLK{}_PW", gclk.tile_wire_prefix(), gclk.z);

    let route_image = crate::route::route_device_design(&lowered, &arch, &arch_path, &cil)?;
    let logical_clock_pips = route_image
        .pips
        .iter()
        .filter(|pip| pip.net_name == logical_clock_net.name)
        .collect::<Vec<_>>();

    assert!(
        logical_clock_pips
            .iter()
            .any(|pip| { pip.from_net == expected_from && clock_route_or_sink_wire(&pip.to_net) })
    );
    assert!(
        logical_clock_pips
            .iter()
            .any(|pip| { clock_route_wire(&pip.from_net) && is_clock_sink_wire_name(&pip.to_net) })
    );
    assert!(
        logical_clock_pips
            .iter()
            .all(|pip| clock_route_or_sink_wire(&pip.to_net))
    );

    Ok(())
}

fn clock_route_wire(raw: &str) -> bool {
    is_clock_distribution_wire_name(raw)
        || is_long_wire_name(raw)
        || is_hex_like_wire_name(raw)
        || is_directional_channel_wire_name(raw)
        || is_pad_stub_wire_name(raw)
}

fn clock_route_or_sink_wire(raw: &str) -> bool {
    clock_route_wire(raw) || is_clock_sink_wire_name(raw)
}

#[test]
fn lowering_uses_cluster_slot_to_select_slice_site_name_when_cil_is_available() -> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let arch = load_arch(&arch_path)?;
    let cil = load_cil(&cil_path)?;
    let Some((x, y)) = arch.logic_sites().into_iter().next() else {
        return Ok(());
    };
    let design = Design {
        name: "slot-lowering".to_string(),
        cells: vec![
            Cell::lut("u0", "LUT4")
                .with_output("O", "n0")
                .in_cluster("clb0"),
        ],
        nets: vec![Net::new("n0").with_driver(Endpoint::cell("u0", "O"))],
        clusters: vec![
            Cluster::logic("clb0")
                .with_member("u0")
                .with_capacity(1)
                .at_slot(x, y, 1),
        ],
        ..Design::default()
    };

    let lowered = lower_design(design, &arch, Some(&cil), &[])?;
    let logic = lowered
        .cells
        .iter()
        .find(|cell| !cell.synthetic && cell.site_kind == SiteKind::LogicSlice)
        .expect("logic cell");
    assert_eq!(logic.site_name, "S1");
    assert_eq!(logic.z, 1);

    Ok(())
}

#[test]
fn lowering_and_device_router_preserve_logical_route_guidance_when_resources_are_available()
-> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let arch = load_arch(&arch_path)?;
    let cil = load_cil(&cil_path)?;
    let Some((src, dst, direct_guide, detour_guide)) = find_guided_pair(&arch) else {
        return Ok(());
    };

    let design_direct = guided_logic_design(src, dst, &direct_guide);
    let design_detour = guided_logic_design(src, dst, &detour_guide);
    let lowered_direct = lower_design(design_direct, &arch, Some(&cil), &[])?;
    let lowered_detour = lower_design(design_detour, &arch, Some(&cil), &[])?;

    assert_ne!(
        lowered_direct.nets[0].guide_tiles,
        lowered_detour.nets[0].guide_tiles
    );
    assert_ne!(
        lowered_direct.nets[0].sink_guides[0].tiles,
        lowered_detour.nets[0].sink_guides[0].tiles
    );

    let route_direct = route_device_design(&lowered_direct, &arch, &arch_path, &cil)?;
    let route_detour = route_device_design(&lowered_detour, &arch, &arch_path, &cil)?;

    let direct_pips = route_direct
        .pips
        .iter()
        .map(|pip| (pip.x, pip.y, pip.from_net.as_str(), pip.to_net.as_str()))
        .collect::<Vec<_>>();
    let detour_pips = route_detour
        .pips
        .iter()
        .map(|pip| (pip.x, pip.y, pip.from_net.as_str(), pip.to_net.as_str()))
        .collect::<Vec<_>>();
    assert_ne!(direct_pips, detour_pips);

    let bitstream_direct = run_bitgen(
        with_route_pips(guided_logic_design(src, dst, &direct_guide), &route_direct),
        &BitgenOptions {
            arch_name: Some(arch.name.clone()),
            arch_path: Some(arch_path.clone()),
            cil_path: Some(cil_path.clone()),
            cil: Some(cil.clone()),
            device_design: Some(lowered_direct),
            route_image: Some(route_direct),
        },
    )?
    .value;
    let bitstream_detour = run_bitgen(
        with_route_pips(guided_logic_design(src, dst, &detour_guide), &route_detour),
        &BitgenOptions {
            arch_name: Some(arch.name.clone()),
            arch_path: Some(arch_path),
            cil_path: Some(cil_path),
            cil: Some(cil.clone()),
            device_design: Some(lowered_detour),
            route_image: Some(route_detour),
        },
    )?
    .value;

    assert_ne!(bitstream_direct.sha256, bitstream_detour.sha256);
    Ok(())
}

#[test]
fn ordered_sink_guides_prevent_shortcuts_that_only_match_the_guide_tile_set() -> Result<()> {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&repo_root()).ok() else {
        return Ok(());
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return Ok(());
    }

    let arch = load_arch(&arch_path)?;
    let cil = load_cil(&cil_path)?;
    let Some((src, dst, direct_guide, ordered_detour_guide)) = find_order_sensitive_pair(&arch)
    else {
        return Ok(());
    };

    let lowered = lower_design(
        guided_logic_design(src, dst, &ordered_detour_guide),
        &arch,
        Some(&cil),
        &[],
    )?;
    let route = route_device_design(&lowered, &arch, &arch_path, &cil)?;

    let direct_tiles = direct_guide.into_iter().collect::<BTreeSet<_>>();
    let detour_only_tiles = ordered_detour_guide
        .into_iter()
        .filter(|tile| !direct_tiles.contains(tile))
        .collect::<BTreeSet<_>>();
    assert!(!detour_only_tiles.is_empty());

    let routed_tiles = route
        .pips
        .iter()
        .map(|pip| (pip.x, pip.y))
        .collect::<BTreeSet<_>>();
    assert!(
        detour_only_tiles
            .iter()
            .any(|tile| routed_tiles.contains(tile))
    );

    Ok(())
}
