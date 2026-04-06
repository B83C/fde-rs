use super::{PackOptions, run};
use crate::ir::{Cell, Design, Endpoint, Net};
use anyhow::Result;
use std::collections::BTreeSet;

fn pack_design() -> Design {
    Design {
        name: "pack-mini".to_string(),
        cells: vec![
            Cell::lut("lut_ff_driver", "LUT4").with_output("O", "d_net"),
            Cell::ff("reg0", "DFFHQ")
                .with_input("D", "d_net")
                .with_output("Q", "q_net"),
            Cell::lut("lut_a", "LUT4")
                .with_input("A", "q_net")
                .with_output("O", "fanout"),
            Cell::lut("lut_b", "LUT4").with_input("A", "fanout"),
        ],
        nets: vec![
            Net::new("d_net")
                .with_driver(Endpoint::cell("lut_ff_driver", "O"))
                .with_sink(Endpoint::cell("reg0", "D")),
            Net::new("q_net")
                .with_driver(Endpoint::cell("reg0", "Q"))
                .with_sink(Endpoint::cell("lut_a", "A")),
            Net::new("fanout")
                .with_driver(Endpoint::cell("lut_a", "O"))
                .with_sink(Endpoint::cell("lut_b", "A")),
        ],
        ..Design::default()
    }
}

fn assert_supported_slice_shapes(packed: &Design) {
    for cluster in &packed.clusters {
        let mut kinds = BTreeSet::new();
        let mut pair_count = 0usize;
        for member in &cluster.members {
            let cell = packed
                .cells
                .iter()
                .find(|cell| cell.name == *member)
                .expect("cluster member exists");
            kinds.insert(if cell.is_lut() {
                "lut"
            } else if cell.is_sequential() {
                "ff"
            } else {
                "other"
            });
            if cell.is_sequential() {
                let d_net = cell
                    .inputs
                    .iter()
                    .find(|pin| pin.port.eq_ignore_ascii_case("D"))
                    .map(|pin| pin.net.as_str());
                if let Some(d_net) = d_net
                    && cluster.members.iter().any(|other| {
                        packed
                            .cells
                            .iter()
                            .find(|candidate| candidate.name == *other)
                            .is_some_and(|candidate| {
                                candidate.is_lut()
                                    && candidate.outputs.iter().any(|pin| pin.net == d_net)
                            })
                    })
                {
                    pair_count += 1;
                }
            }
        }
        let supported = match (
            kinds.contains("lut"),
            kinds.contains("ff"),
            cluster.members.len(),
        ) {
            (true, true, 2) => pair_count == 1,
            (true, true, 4) => pair_count == 2,
            (true, false, 1 | 2) => true,
            (false, true, 1 | 2) => true,
            (false, false, 1) => true,
            _ => false,
        };
        assert!(
            supported,
            "unsupported slice shape in {:?}: {:?}",
            cluster.name, cluster.members
        );
    }
}

#[test]
fn pack_pairs_lut_with_sequential_d_input_and_respects_capacity() -> Result<()> {
    let packed = run(
        pack_design(),
        &PackOptions {
            family: Some("fdp3".to_string()),
            capacity: 2,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.stage, "packed");
    assert_eq!(packed.metadata.family, "fdp3");
    assert_eq!(packed.clusters.len(), 2);
    assert!(
        packed
            .clusters
            .iter()
            .all(|cluster| cluster.members.len() <= 2)
    );

    let ff_cluster = packed
        .clusters
        .iter()
        .find(|cluster| cluster.members.iter().any(|member| member == "reg0"))
        .expect("cluster containing reg0");
    assert!(
        ff_cluster
            .members
            .iter()
            .any(|member| member == "lut_ff_driver")
    );

    let remaining_cluster = packed
        .clusters
        .iter()
        .find(|cluster| cluster.name != ff_cluster.name)
        .expect("remaining cluster");
    assert_eq!(
        remaining_cluster.members,
        vec!["lut_a".to_string(), "lut_b".to_string()]
    );

    for cell in &packed.cells {
        assert!(
            cell.cluster.is_some(),
            "expected packed cluster for {}",
            cell.name
        );
    }

    Ok(())
}

#[test]
fn pack_scales_across_multiple_independent_lut_ff_pairs() -> Result<()> {
    let mut design = Design {
        name: "pack-many".to_string(),
        ..Design::default()
    };
    for index in 0..8 {
        let lut_name = format!("lut_{index}");
        let ff_name = format!("ff_{index}");
        let net_name = format!("d_net_{index}");
        design
            .cells
            .push(Cell::lut(lut_name.clone(), "LUT4").with_output("O", net_name.clone()));
        design
            .cells
            .push(Cell::ff(ff_name.clone(), "DFFHQ").with_input("D", net_name.clone()));
        design.nets.push(
            Net::new(net_name)
                .with_driver(Endpoint::cell(lut_name, "O"))
                .with_sink(Endpoint::cell(ff_name, "D")),
        );
    }

    let packed = run(
        design,
        &PackOptions {
            capacity: 2,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 8);
    for index in 0..8 {
        let lut_name = format!("lut_{index}");
        let ff_name = format!("ff_{index}");
        let cluster = packed
            .clusters
            .iter()
            .find(|cluster| cluster.members.iter().any(|member| member == &ff_name))
            .expect("matching ff cluster");
        assert_eq!(cluster.members.len(), 2);
        assert!(cluster.members.iter().any(|member| member == &lut_name));
    }

    Ok(())
}

#[test]
fn pack_assigns_block_ram_cells_to_dedicated_singleton_clusters() -> Result<()> {
    let design = Design {
        name: "pack-bram".to_string(),
        cells: vec![
            Cell::new(
                "ram0",
                crate::domain::CellKind::BlockRam,
                "BLOCKRAM_SINGLE_PORT",
            )
            .with_input("CLK", "clk")
            .with_output("DO0", "q"),
            Cell::lut("lut0", "LUT4").with_input("A", "q"),
        ],
        nets: vec![
            Net::new("clk").with_driver(Endpoint::port("clk", "IN")),
            Net::new("q")
                .with_driver(Endpoint::cell("ram0", "DO0"))
                .with_sink(Endpoint::cell("lut0", "A")),
        ],
        ..Design::default()
    };

    let packed = run(design, &PackOptions::default())?.value;
    let bram_cluster = packed
        .clusters
        .iter()
        .find(|cluster| cluster.members.iter().any(|member| member == "ram0"))
        .expect("bram cluster");
    assert_eq!(bram_cluster.kind, crate::domain::ClusterKind::BlockRam);
    assert_eq!(bram_cluster.capacity, 1);
    assert_eq!(bram_cluster.members, vec!["ram0".to_string()]);

    Ok(())
}

#[test]
fn pack_can_fill_four_slot_cluster_along_connected_chain() -> Result<()> {
    let design = Design {
        name: "pack-chain".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "net0"),
            Cell::ff("ff0", "DFFHQ")
                .with_input("D", "net0")
                .with_output("Q", "net1"),
            Cell::lut("lut1", "LUT4")
                .with_input("A", "net1")
                .with_output("O", "net2"),
            Cell::ff("ff1", "DFFHQ").with_input("D", "net2"),
        ],
        nets: vec![
            Net::new("net0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("net1")
                .with_driver(Endpoint::cell("ff0", "Q"))
                .with_sink(Endpoint::cell("lut1", "A")),
            Net::new("net2")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("ff1", "D")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            family: Some("fdp3".to_string()),
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 1);
    assert_eq!(packed.clusters[0].members.len(), 4);
    assert_eq!(
        packed.clusters[0].members,
        vec![
            "lut0".to_string(),
            "ff0".to_string(),
            "lut1".to_string(),
            "ff1".to_string(),
        ]
    );

    Ok(())
}

#[test]
fn pack_pairs_connected_lut_ff_lanes_before_plain_encounter_order() -> Result<()> {
    let design = Design {
        name: "pack-connected-lut-ff-lanes".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "d0"),
            Cell::ff("ff0", "DFFHQ")
                .with_input("D", "d0")
                .with_output("Q", "q0"),
            Cell::lut("lut1", "LUT4").with_output("O", "d1"),
            Cell::ff("ff1", "DFFHQ")
                .with_input("D", "d1")
                .with_output("Q", "q1"),
            Cell::lut("lut2", "LUT4")
                .with_input("A", "q0")
                .with_output("O", "d2"),
            Cell::ff("ff2", "DFFHQ")
                .with_input("D", "d2")
                .with_output("Q", "q2"),
            Cell::lut("lut3", "LUT4")
                .with_input("A", "q1")
                .with_output("O", "d3"),
            Cell::ff("ff3", "DFFHQ").with_input("D", "d3"),
        ],
        nets: vec![
            Net::new("d0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("q0")
                .with_driver(Endpoint::cell("ff0", "Q"))
                .with_sink(Endpoint::cell("lut2", "A")),
            Net::new("d1")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("ff1", "D")),
            Net::new("q1")
                .with_driver(Endpoint::cell("ff1", "Q"))
                .with_sink(Endpoint::cell("lut3", "A")),
            Net::new("d2")
                .with_driver(Endpoint::cell("lut2", "O"))
                .with_sink(Endpoint::cell("ff2", "D")),
            Net::new("d3")
                .with_driver(Endpoint::cell("lut3", "O"))
                .with_sink(Endpoint::cell("ff3", "D")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 2);
    assert_eq!(
        packed.clusters[0].members,
        vec![
            "lut0".to_string(),
            "ff0".to_string(),
            "lut2".to_string(),
            "ff2".to_string(),
        ]
    );
    assert_eq!(
        packed.clusters[1].members,
        vec![
            "lut1".to_string(),
            "ff1".to_string(),
            "lut3".to_string(),
            "ff3".to_string(),
        ]
    );

    Ok(())
}

#[test]
fn pack_pairs_connected_lut_lanes_before_plain_encounter_order() -> Result<()> {
    let design = Design {
        name: "pack-connected-lut-lanes".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4")
                .with_output("O", "n0")
                .with_input("A", "a0"),
            Cell::lut("lut1", "LUT4")
                .with_output("O", "n1")
                .with_input("A", "a1"),
            Cell::lut("lut2", "LUT4").with_input("A", "n0"),
            Cell::lut("lut3", "LUT4").with_input("A", "n1"),
        ],
        nets: vec![
            Net::new("n0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("lut2", "A")),
            Net::new("n1")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("lut3", "A")),
            Net::new("a0").with_sink(Endpoint::cell("lut0", "A")),
            Net::new("a1").with_sink(Endpoint::cell("lut1", "A")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 2);
    assert_eq!(
        packed.clusters[0].members,
        vec!["lut0".to_string(), "lut2".to_string()]
    );
    assert_eq!(
        packed.clusters[1].members,
        vec!["lut1".to_string(), "lut3".to_string()]
    );

    Ok(())
}

#[test]
fn pack_respects_slice_shape_limits_when_greedy_fill_expands() -> Result<()> {
    let design = Design {
        name: "pack-shape".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "net0"),
            Cell::ff("ff0", "DFFHQ")
                .with_input("D", "net0")
                .with_output("Q", "net1"),
            Cell::lut("lut1", "LUT4")
                .with_input("A", "net1")
                .with_output("O", "net2"),
            Cell::lut("lut2", "LUT4").with_input("A", "net2"),
            Cell::lut("lut3", "LUT4").with_input("A", "net2"),
        ],
        nets: vec![
            Net::new("net0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("net1")
                .with_driver(Endpoint::cell("ff0", "Q"))
                .with_sink(Endpoint::cell("lut1", "A")),
            Net::new("net2")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("lut2", "A"))
                .with_sink(Endpoint::cell("lut3", "A")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            family: Some("fdp3".to_string()),
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 3);
    assert_supported_slice_shapes(&packed);

    Ok(())
}

#[test]
fn pack_can_fill_slice_with_unconnected_pure_lut_lane() -> Result<()> {
    let design = Design {
        name: "pack-unconnected-luts".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "n0"),
            Cell::lut("lut1", "LUT4").with_output("O", "n1"),
        ],
        nets: vec![Net::new("n0"), Net::new("n1")],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 1);
    assert_eq!(
        packed.clusters[0].members,
        vec!["lut0".to_string(), "lut1".to_string()]
    );

    Ok(())
}

#[test]
fn pack_does_not_mix_lut_ff_lane_with_unconnected_pure_lut_lane() -> Result<()> {
    let design = Design {
        name: "pack-no-unconnected-fill".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "d0"),
            Cell::ff("ff0", "DFFHQ").with_input("D", "d0"),
            Cell::lut("lut1", "LUT4").with_output("O", "n1"),
        ],
        nets: vec![
            Net::new("d0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("n1"),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 2);
    assert_eq!(
        packed.clusters[0].members,
        vec!["lut0".to_string(), "ff0".to_string()]
    );
    assert_eq!(packed.clusters[1].members, vec!["lut1".to_string()]);

    Ok(())
}

#[test]
fn pack_keeps_incompatible_ff_control_sets_in_separate_clusters() -> Result<()> {
    let design = Design {
        name: "pack-control-set-split".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "d0"),
            Cell::ff("ff0", "DFFHQ").with_input("D", "d0"),
            Cell::lut("lut1", "LUT4").with_output("O", "d1"),
            Cell::ff("ff1", "EDFFHQ")
                .with_input("D", "d1")
                .with_input("E", "ce"),
        ],
        nets: vec![
            Net::new("d0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("d1")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("ff1", "D")),
            Net::new("ce").with_sink(Endpoint::cell("ff1", "E")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 2);
    assert!(
        packed
            .clusters
            .iter()
            .all(|cluster| cluster.members.len() == 2)
    );

    Ok(())
}

#[test]
fn pack_merges_matching_ff_control_sets() -> Result<()> {
    let design = Design {
        name: "pack-control-set-merge".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "d0"),
            Cell::ff("ff0", "EDFFHQ")
                .with_input("D", "d0")
                .with_input("E", "ce"),
            Cell::lut("lut1", "LUT4").with_output("O", "d1"),
            Cell::ff("ff1", "EDFFHQ")
                .with_input("D", "d1")
                .with_input("E", "ce"),
        ],
        nets: vec![
            Net::new("d0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("d1")
                .with_driver(Endpoint::cell("lut1", "O"))
                .with_sink(Endpoint::cell("ff1", "D")),
            Net::new("ce")
                .with_sink(Endpoint::cell("ff0", "E"))
                .with_sink(Endpoint::cell("ff1", "E")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 1);
    assert_eq!(packed.clusters[0].members.len(), 4);
    assert_supported_slice_shapes(&packed);

    Ok(())
}

#[test]
fn pack_does_not_mix_lut_ff_pair_with_plain_lut() -> Result<()> {
    let design = Design {
        name: "pack-no-lutff-lut-mix".to_string(),
        cells: vec![
            Cell::lut("lut0", "LUT4").with_output("O", "d0"),
            Cell::ff("ff0", "DFFHQ")
                .with_input("D", "d0")
                .with_output("Q", "q0"),
            Cell::lut("lut1", "LUT4")
                .with_input("A", "q0")
                .with_output("O", "n1"),
        ],
        nets: vec![
            Net::new("d0")
                .with_driver(Endpoint::cell("lut0", "O"))
                .with_sink(Endpoint::cell("ff0", "D")),
            Net::new("q0")
                .with_driver(Endpoint::cell("ff0", "Q"))
                .with_sink(Endpoint::cell("lut1", "A")),
        ],
        ..Design::default()
    };

    let packed = run(
        design,
        &PackOptions {
            capacity: 4,
            ..PackOptions::default()
        },
    )?
    .value;

    assert_eq!(packed.clusters.len(), 2);
    assert_supported_slice_shapes(&packed);

    Ok(())
}
