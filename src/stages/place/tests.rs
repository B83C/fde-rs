use super::{
    PlaceMode, PlaceOptions,
    cost::{PlacementEvaluator, evaluate},
    graph::build_cluster_graph,
    model::{PlacementModel, Point},
    run,
};
use crate::{
    ir::{Cell, CellPin, Cluster, ClusterId, Design, Endpoint, Net, Port},
    resource::{Arch, DelayModel},
};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn mini_arch() -> Arch {
    Arch {
        name: "mini".to_string(),
        width: 6,
        height: 6,
        slices_per_tile: 2,
        lut_inputs: 4,
        wire_r: 0.04,
        wire_c: 0.03,
        ..Arch::default()
    }
}

fn mini_delay() -> DelayModel {
    DelayModel {
        name: "clb2clb".to_string(),
        width: 6,
        height: 6,
        values: vec![
            vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5],
            vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
            vec![0.2, 0.3, 0.4, 0.5, 0.6, 0.7],
            vec![0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            vec![0.4, 0.5, 0.6, 0.7, 0.8, 0.9],
            vec![0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
        ],
    }
}

fn synthetic_arch(width: usize, height: usize) -> Arch {
    Arch {
        name: format!("synthetic-{width}x{height}"),
        width,
        height,
        slices_per_tile: 2,
        lut_inputs: 4,
        wire_r: 0.04,
        wire_c: 0.03,
        ..Arch::default()
    }
}

fn synthetic_delay(width: usize, height: usize) -> DelayModel {
    let values = (0..height)
        .map(|dy| {
            (0..width)
                .map(|dx| 0.05 * (dx + dy) as f64)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    DelayModel {
        name: format!("synthetic-{width}x{height}"),
        width,
        height,
        values,
    }
}

fn clustered_design() -> Design {
    Design {
        name: "place-mini".to_string(),
        ports: vec![Port::input("in").at(0, 2), Port::output("out").at(5, 2)],
        cells: vec![
            Cell::lut("u0", "LUT4")
                .with_input("A", "in_net")
                .with_output("O", "mid0")
                .in_cluster("clb0"),
            Cell::lut("u1", "LUT4")
                .with_input("A", "mid0")
                .with_output("O", "mid1")
                .in_cluster("clb1"),
            Cell::lut("u2", "LUT4")
                .with_input("A", "mid1")
                .with_output("O", "out_net")
                .in_cluster("clb2"),
        ],
        nets: vec![
            Net::new("in_net")
                .with_driver(Endpoint::port("in", "IN"))
                .with_sink(Endpoint::cell("u0", "A")),
            Net::new("mid0")
                .with_driver(Endpoint::cell("u0", "O"))
                .with_sink(Endpoint::cell("u1", "A")),
            Net::new("mid1")
                .with_driver(Endpoint::cell("u1", "O"))
                .with_sink(Endpoint::cell("u2", "A")),
            Net::new("out_net")
                .with_driver(Endpoint::cell("u2", "O"))
                .with_sink(Endpoint::port("out", "OUT")),
        ],
        clusters: vec![
            Cluster::logic("clb0").with_member("u0").with_capacity(1),
            Cluster::logic("clb1").with_member("u1").with_capacity(1),
            Cluster::logic("clb2").with_member("u2").with_capacity(1),
        ],
        ..Design::default()
    }
}

fn placed_coordinates(design: &Design) -> Vec<(String, usize, usize)> {
    let mut coords = design
        .clusters
        .iter()
        .map(|cluster| {
            (
                cluster.name.clone(),
                cluster.x.unwrap_or(usize::MAX),
                cluster.y.unwrap_or(usize::MAX),
            )
        })
        .collect::<Vec<_>>();
    coords.sort();
    coords
}

fn placed_sites(design: &Design) -> Vec<(String, usize, usize, usize)> {
    let mut coords = design
        .clusters
        .iter()
        .map(|cluster| {
            (
                cluster.name.clone(),
                cluster.x.unwrap_or(usize::MAX),
                cluster.y.unwrap_or(usize::MAX),
                cluster.z.unwrap_or(usize::MAX),
            )
        })
        .collect::<Vec<_>>();
    coords.sort();
    coords
}

fn connected_pair_design() -> Design {
    Design {
        name: "place-pair".to_string(),
        cells: vec![
            Cell::lut("src", "LUT4")
                .with_output("O", "link")
                .in_cluster("clb0"),
            Cell::lut("dst", "LUT4")
                .with_input("A", "link")
                .in_cluster("clb1"),
        ],
        nets: vec![
            Net::new("link")
                .with_driver(Endpoint::cell("src", "O"))
                .with_sink(Endpoint::cell("dst", "A")),
        ],
        clusters: vec![
            Cluster::logic("clb0").with_member("src").with_capacity(1),
            Cluster::logic("clb1").with_member("dst").with_capacity(1),
        ],
        ..Design::default()
    }
}

fn overfull_single_tile_design() -> Design {
    Design {
        name: "place-single-tile".to_string(),
        cells: vec![
            Cell::lut("u0", "LUT4")
                .with_output("O", "n0")
                .in_cluster("clb0"),
            Cell::lut("u1", "LUT4")
                .with_input("A", "n0")
                .in_cluster("clb1"),
        ],
        nets: vec![
            Net::new("n0")
                .with_driver(Endpoint::cell("u0", "O"))
                .with_sink(Endpoint::cell("u1", "A")),
        ],
        clusters: vec![
            Cluster::logic("clb0").with_member("u0").with_capacity(1),
            Cluster::logic("clb1").with_member("u1").with_capacity(1),
        ],
        ..Design::default()
    }
}

fn large_grid_design(width: usize, height: usize) -> Design {
    let mut cells = Vec::new();
    let mut clusters = Vec::new();
    let mut nets = Vec::new();
    let mut input_nets = vec![vec![Option::<String>::None; width]; height];

    for (y, row_input_nets) in input_nets.iter_mut().enumerate().take(height) {
        for x in 0..width {
            let cell_name = format!("u_{x}_{y}");
            let cluster_name = format!("clb_{x}_{y}");
            let mut inputs = Vec::new();
            if x > 0
                && let Some(net) = &row_input_nets[x]
            {
                inputs.push(CellPin::new("A", net.clone()));
            }
            if y > 0 {
                let net = format!("v_{x}_{}", y - 1);
                inputs.push(CellPin::new("B", net));
            }

            let mut outputs = Vec::new();
            if x + 1 < width {
                let net_name = format!("h_{x}_{y}");
                outputs.push(CellPin::new("OX", net_name.clone()));
                nets.push(
                    Net::new(net_name.clone())
                        .with_driver(Endpoint::cell(cell_name.clone(), "OX"))
                        .with_sink(Endpoint::cell(format!("u_{}_{}", x + 1, y), "A")),
                );
                row_input_nets[x + 1] = Some(net_name);
            }
            if y + 1 < height {
                let net_name = format!("v_{x}_{y}");
                outputs.push(CellPin::new("OY", net_name.clone()));
                nets.push(
                    Net::new(net_name)
                        .with_driver(Endpoint::cell(cell_name.clone(), "OY"))
                        .with_sink(Endpoint::cell(format!("u_{x}_{}", y + 1), "B")),
                );
            }

            if x + 2 < width && y + 1 < height && (x + y) % 3 == 0 {
                let net_name = format!("d_{x}_{y}");
                outputs.push(CellPin::new("OD", net_name.clone()));
                nets.push(
                    Net::new(net_name)
                        .with_driver(Endpoint::cell(cell_name.clone(), "OD"))
                        .with_sink(Endpoint::cell(format!("u_{}_{}", x + 2, y + 1), "C")),
                );
            }

            cells.push(Cell {
                inputs,
                outputs,
                ..Cell::lut(cell_name.clone(), "LUT4").in_cluster(cluster_name.clone())
            });
            clusters.push(
                Cluster::logic(cluster_name)
                    .with_member(cell_name)
                    .with_capacity(1),
            );
        }
    }

    Design {
        name: format!("large-grid-{width}x{height}"),
        cells,
        nets,
        clusters,
        ..Design::default()
    }
}

fn fixed_cluster_design() -> Design {
    let mut design = connected_pair_design();
    if let Some(cluster) = design
        .clusters
        .iter_mut()
        .find(|cluster| cluster.name == "clb0")
    {
        *cluster = cluster.clone().fixed_at(1, 1);
    }
    design
}

fn apply_net_criticality(design: &mut Design, hot_nets: &[&str], default: f64, hot: f64) {
    let hot_nets = hot_nets.iter().copied().collect::<BTreeSet<_>>();
    for net in &mut design.nets {
        net.criticality = if hot_nets.contains(net.name.as_str()) {
            hot
        } else {
            default
        };
    }
}

fn placement_vec(
    model: &PlacementModel,
    placements: &[(ClusterId, (usize, usize))],
) -> Vec<Option<Point>> {
    let mut resolved = model.fixed_placements();
    for &(cluster_id, (x, y)) in placements {
        resolved[cluster_id.index()] = Some(Point::new(x, y));
    }
    resolved
}

#[test]
fn placement_is_seed_stable_and_legal_in_both_modes() -> Result<()> {
    for mode in [PlaceMode::BoundingBox, PlaceMode::TimingDriven] {
        let options = PlaceOptions {
            arch: mini_arch().into(),
            delay: Some(mini_delay().into()),
            constraints: Vec::new().into(),
            mode,
            seed: 0xCAFE_BABE,
        };
        let first = run(clustered_design(), &options)?.value;
        let second = run(clustered_design(), &options)?.value;

        let first_coords = placed_coordinates(&first);
        let second_coords = placed_coordinates(&second);
        assert_eq!(
            first_coords, second_coords,
            "placement should be deterministic"
        );

        let unique_sites = first_coords
            .iter()
            .map(|(_, x, y)| (*x, *y))
            .collect::<BTreeSet<_>>();
        assert!(unique_sites.len() <= first.clusters.len());
        assert!(
            first_coords
                .iter()
                .all(|(_, x, y)| *x > 0 && *x < 5 && *y > 0 && *y < 5)
        );
    }

    Ok(())
}

#[test]
fn strongly_connected_pair_is_placed_adjacent() -> Result<()> {
    for mode in [PlaceMode::BoundingBox, PlaceMode::TimingDriven] {
        let placed = run(
            connected_pair_design(),
            &PlaceOptions {
                arch: mini_arch().into(),
                delay: Some(mini_delay().into()),
                constraints: Vec::new().into(),
                mode,
                seed: 7,
            },
        )?
        .value;

        let coords = placed_coordinates(&placed);
        let lhs = (coords[0].1, coords[0].2);
        let rhs = (coords[1].1, coords[1].2);
        assert!(
            super::manhattan(lhs, rhs) <= 1,
            "expected colocated-or-adjacent placement in {mode:?}"
        );
    }

    Ok(())
}

#[test]
fn fixed_clusters_keep_their_requested_site() -> Result<()> {
    let placed = run(
        fixed_cluster_design(),
        &PlaceOptions {
            arch: mini_arch().into(),
            delay: Some(mini_delay().into()),
            constraints: Vec::new().into(),
            mode: PlaceMode::TimingDriven,
            seed: 99,
        },
    )?
    .value;
    let fixed = placed
        .clusters
        .iter()
        .find(|cluster| cluster.name == "clb0")
        .ok_or_else(|| anyhow::anyhow!("missing fixed cluster"))?;
    assert_eq!((fixed.x, fixed.y), (Some(1), Some(1)));
    assert!(fixed.fixed);
    Ok(())
}

#[test]
fn placement_uses_multiple_slice_slots_per_tile_when_capacity_allows() -> Result<()> {
    let placed = run(
        overfull_single_tile_design(),
        &PlaceOptions {
            arch: synthetic_arch(1, 1).into(),
            delay: None,
            constraints: Arc::from([]),
            mode: PlaceMode::BoundingBox,
            seed: 0xA11CE,
        },
    )?
    .value;

    let sites = placed_sites(&placed);
    assert_eq!(
        sites,
        vec![("clb0".to_string(), 0, 0, 0), ("clb1".to_string(), 0, 0, 1),]
    );

    Ok(())
}

#[test]
fn timing_objective_penalizes_stretched_critical_chain_more_strongly() {
    let mut design = clustered_design();
    apply_net_criticality(&mut design, &["mid0", "mid1"], 0.1, 1.0);
    let graph = build_cluster_graph(&design);
    let model = PlacementModel::from_design(&design);
    let compact = placement_vec(
        &model,
        &[
            (ClusterId::new(0), (1usize, 2usize)),
            (ClusterId::new(1), (2usize, 2usize)),
            (ClusterId::new(2), (3usize, 2usize)),
        ],
    );
    let stretched = placement_vec(
        &model,
        &[
            (ClusterId::new(0), (1usize, 1usize)),
            (ClusterId::new(1), (3usize, 3usize)),
            (ClusterId::new(2), (4usize, 4usize)),
        ],
    );

    let bounding_gap = evaluate(
        &model,
        &graph,
        &stretched,
        &mini_arch(),
        Some(&mini_delay()),
        PlaceMode::BoundingBox,
    )
    .total
        - evaluate(
            &model,
            &graph,
            &compact,
            &mini_arch(),
            Some(&mini_delay()),
            PlaceMode::BoundingBox,
        )
        .total;
    let timing_gap = evaluate(
        &model,
        &graph,
        &stretched,
        &mini_arch(),
        Some(&mini_delay()),
        PlaceMode::TimingDriven,
    )
    .total
        - evaluate(
            &model,
            &graph,
            &compact,
            &mini_arch(),
            Some(&mini_delay()),
            PlaceMode::TimingDriven,
        )
        .total;

    assert!(timing_gap > bounding_gap);
}

#[test]
fn incremental_evaluator_matches_full_recompute_for_move_and_swap() {
    let mut design = clustered_design();
    apply_net_criticality(&mut design, &["mid0", "mid1"], 0.2, 1.0);

    let graph = build_cluster_graph(&design);
    let model = PlacementModel::from_design(&design);
    let placements = placement_vec(
        &model,
        &[
            (ClusterId::new(0), (1usize, 2usize)),
            (ClusterId::new(1), (2usize, 2usize)),
            (ClusterId::new(2), (4usize, 3usize)),
        ],
    );
    let arch = mini_arch();
    let delay = mini_delay();
    let mode = PlaceMode::TimingDriven;
    let mut evaluator = PlacementEvaluator::new_from_positions(
        &model,
        &graph,
        placements.clone(),
        &arch,
        Some(&delay),
        mode,
    );

    let clb0 = ClusterId::new(0);
    let clb1 = ClusterId::new(1);
    let clb2 = ClusterId::new(2);

    let move_updates = vec![(clb1, (3usize, 1usize))];
    let move_candidate = evaluator.evaluate_candidate(&move_updates);
    let repeated_move_candidate = evaluator.evaluate_candidate(&move_updates);
    let mut moved = placements.clone();
    moved[clb1.index()] = Some(Point::new(3, 1));
    let moved_metrics = evaluate(&model, &graph, &moved, &arch, Some(&delay), mode);
    assert_metrics_close(move_candidate.metrics(), &moved_metrics);
    assert_metrics_close(repeated_move_candidate.metrics(), &moved_metrics);

    evaluator.apply_candidate(move_candidate);
    assert_metrics_close(evaluator.metrics(), &moved_metrics);

    let swap_updates = vec![(clb0, (4usize, 3usize)), (clb2, (1usize, 2usize))];
    let swap_candidate = evaluator.evaluate_candidate(&swap_updates);
    let mut swapped = moved.clone();
    swapped[clb0.index()] = Some(Point::new(4, 3));
    swapped[clb2.index()] = Some(Point::new(1, 2));
    let swapped_metrics = evaluate(&model, &graph, &swapped, &arch, Some(&delay), mode);
    assert_metrics_close(swap_candidate.metrics(), &swapped_metrics);

    evaluator.apply_candidate(swap_candidate);
    assert_metrics_close(evaluator.metrics(), &swapped_metrics);
}

#[test]
fn large_synthetic_design_places_legally_and_deterministically() -> Result<()> {
    let design = large_grid_design(9, 9);
    let arch = synthetic_arch(14, 14);
    let slices_per_tile = arch.slices_per_tile;
    let delay = synthetic_delay(arch.width, arch.height);
    let options = PlaceOptions {
        arch: arch.into(),
        delay: Some(delay.into()),
        constraints: Vec::new().into(),
        mode: PlaceMode::TimingDriven,
        seed: 0xC0FFEE,
    };

    let placed_a = run(design.clone(), &options)?.value;
    let placed_b = run(design, &options)?.value;
    let coords_a = placed_coordinates(&placed_a);
    let coords_b = placed_coordinates(&placed_b);

    assert_eq!(coords_a, coords_b);

    let unique_sites = coords_a
        .iter()
        .map(|(_, x, y)| (*x, *y))
        .collect::<BTreeSet<_>>();
    assert!(unique_sites.len().saturating_mul(slices_per_tile) >= coords_a.len());
    let mut occupancy = BTreeMap::<(usize, usize), usize>::new();
    for (_, x, y) in &coords_a {
        *occupancy.entry((*x, *y)).or_default() += 1;
    }
    assert!(occupancy.values().all(|count| *count <= slices_per_tile));
    assert!(coords_a.iter().all(|(_, x, y)| *x < 14 && *y < 14));

    Ok(())
}

fn assert_metrics_close(lhs: &super::cost::PlacementMetrics, rhs: &super::cost::PlacementMetrics) {
    for (lhs_value, rhs_value) in [
        (lhs.wire_cost, rhs.wire_cost),
        (lhs.congestion_cost, rhs.congestion_cost),
        (lhs.timing_cost, rhs.timing_cost),
        (lhs.locality_cost, rhs.locality_cost),
        (lhs.total, rhs.total),
    ] {
        assert!(
            (lhs_value - rhs_value).abs() < 1e-9,
            "metrics diverged: {lhs_value} vs {rhs_value}"
        );
    }
}
