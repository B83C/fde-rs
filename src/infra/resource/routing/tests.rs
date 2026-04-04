use super::{
    RouteNode, TileSide, WireInterner, build_stitched_components, clock_spine_neighbors,
    load_site_route_defaults, load_site_route_graphs, load_tile_stitch_db, stitched_neighbors,
};
use crate::{
    cil::parse_cil_str,
    resource::{Arch, TileInstance, load_arch},
};
use std::{collections::BTreeMap, fs};
use tempfile::NamedTempFile;

#[test]
fn extracts_default_transmission_bits_from_arch_instances() {
    let cil = parse_cil_str(
        r##"
            <device name="mini">
              <element_library>
                <element name="SW2">
                  <sram_info amount="1">
                    <sram name="EN" default="1"/>
                  </sram_info>
                  <path_info amount="1">
                    <path in="IN" out="OUT"/>
                  </path_info>
                </element>
              </element_library>
              <transmission_library>
                <signal_transmission name="CNTX" type="GSB_CNT"/>
              </transmission_library>
              <tile_library>
                <tile name="CENTER" sram_amount="R1C1">
                  <transmission_info amount="1">
                    <transmission type="CNTX">
                      <site name="GSB_CNT" position="R0C0">
                        <site_sram>
                          <sram basic_cell="sw0" sram_name="EN" local_place="B0W0"/>
                        </site_sram>
                      </site>
                    </transmission>
                  </transmission_info>
                </tile>
              </tile_library>
            </device>
            "##,
    )
    .expect("parse mini cil");
    let arch_xml = r#"
        <architecture>
          <library name="block">
            <cell name="GSB_CNT">
              <contents>
                <instance name="sw0" cellRef="SW2"/>
                <net name="IN">
                  <portRef instanceRef="sw0" name="IN"/>
                </net>
                <net name="OUT">
                  <portRef instanceRef="sw0" name="OUT"/>
                </net>
              </contents>
            </cell>
          </library>
        </architecture>
        "#;
    let file = NamedTempFile::new().expect("temp arch xml");
    fs::write(file.path(), arch_xml).expect("write arch xml");

    let defaults = load_site_route_defaults(file.path(), &cil).expect("load defaults");
    let bits = defaults.get("GSB_CNT").expect("GSB_CNT defaults");

    assert_eq!(bits.len(), 1);
    assert_eq!(bits[0].basic_cell, "sw0");
    assert_eq!(bits[0].sram_name, "EN");
    assert_eq!(bits[0].value, 1);
}

#[test]
fn site_route_graphs_sort_instances_by_name_for_stable_arc_order() {
    let cil = parse_cil_str(
        r##"
            <device name="mini">
              <element_library>
                <element name="SW2">
                  <path_info amount="1">
                    <path in="IN" out="OUT"/>
                  </path_info>
                </element>
              </element_library>
              <transmission_library>
                <signal_transmission name="CNTX" type="GSB_CNT"/>
              </transmission_library>
              <tile_library>
                <tile name="CENTER" sram_amount="R1C1">
                  <transmission_info amount="1">
                    <transmission type="CNTX">
                      <site name="GSB_CNT" position="R0C0"/>
                    </transmission>
                  </transmission_info>
                </tile>
              </tile_library>
            </device>
            "##,
    )
    .expect("parse mini cil");
    let arch_xml = r#"
        <architecture>
          <library name="block">
            <cell name="GSB_CNT">
              <contents>
                <instance name="sw_b" cellRef="SW2"/>
                <instance name="sw_a" cellRef="SW2"/>
                <net name="SRC">
                  <portRef instanceRef="sw_b" name="IN"/>
                  <portRef instanceRef="sw_a" name="IN"/>
                </net>
                <net name="A">
                  <portRef instanceRef="sw_a" name="OUT"/>
                </net>
                <net name="B">
                  <portRef instanceRef="sw_b" name="OUT"/>
                </net>
              </contents>
            </cell>
          </library>
        </architecture>
        "#;
    let file = NamedTempFile::new().expect("temp arch xml");
    fs::write(file.path(), arch_xml).expect("write arch xml");

    let mut wires = WireInterner::default();
    let graphs = load_site_route_graphs(file.path(), &cil, &mut wires).expect("load graphs");
    let graph = graphs.get("GSB_CNT").expect("GSB_CNT graph");
    let src = wires.intern("SRC");
    let indices = graph.adjacency(src);
    assert!(!indices.is_empty(), "SRC adjacency");
    let arcs = indices
        .iter()
        .map(|&index| {
            let arc = &graph.arcs[index];
            (arc.basic_cell.as_str(), wires.resolve(arc.to))
        })
        .collect::<Vec<_>>();

    assert_eq!(arcs, vec![("sw_a", "A"), ("sw_b", "B")]);
}

#[test]
fn parses_tile_port_stitching_from_minimal_architecture() {
    let xml = r#"
        <architecture>
          <library name="tiles">
            <cell name="LEFT" type="TILE">
              <port name="right" msb="0" lsb="0" side="right"/>
              <contents>
                <net name="LEFT_E0">
                  <portRef name="right0"/>
                </net>
              </contents>
            </cell>
            <cell name="CENTER" type="TILE">
              <port name="left" msb="0" lsb="0" side="left"/>
              <contents>
                <net name="W0">
                  <portRef name="left0"/>
                </net>
              </contents>
            </cell>
          </library>
        </architecture>
        "#;
    let file = NamedTempFile::new().expect("temp arch");
    fs::write(file.path(), xml).expect("write arch");
    let mut wires = WireInterner::default();
    let db = load_tile_stitch_db(file.path(), &mut wires).expect("load stitch db");

    let arch = Arch {
        width: 1,
        height: 2,
        tiles: BTreeMap::from([
            (
                (0, 0),
                TileInstance {
                    name: "L0".to_string(),
                    tile_type: "LEFT".to_string(),
                    logic_x: 0,
                    logic_y: 0,
                    bit_x: 0,
                    bit_y: 0,
                    phy_x: 0,
                    phy_y: 0,
                },
            ),
            (
                (0, 1),
                TileInstance {
                    name: "C0".to_string(),
                    tile_type: "CENTER".to_string(),
                    logic_x: 0,
                    logic_y: 1,
                    bit_x: 0,
                    bit_y: 1,
                    phy_x: 0,
                    phy_y: 1,
                },
            ),
        ]),
        ..Arch::default()
    };

    let node = RouteNode::new(0, 0, wires.intern("LEFT_E0"));
    let neighbors = stitched_neighbors(&db, &arch, &wires, &node);

    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].0, 0);
    assert_eq!(neighbors[0].1, 1);
    assert_eq!(wires.resolve(neighbors[0].2), "W0");
}

#[test]
fn real_arch_stitching_matches_llh_and_edge_port_mappings() {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&std::path::PathBuf::from(
        env!("CARGO_MANIFEST_DIR"),
    ))
    .ok() else {
        return;
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    if !arch_path.exists() {
        return;
    }

    let arch = load_arch(&arch_path).expect("load arch");
    let mut wires = WireInterner::default();
    let db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");

    let right_llh_wire = wires.intern("RIGHT_LLH3");
    let right_llh = stitched_neighbors(&db, &arch, &wires, &RouteNode::new(4, 53, right_llh_wire));
    assert!(
        right_llh
            .iter()
            .any(|&(x, y, wire)| x == 4 && y == 52 && wires.resolve(wire) == "LLH4")
    );

    let right_h6_wire = wires.intern("RIGHT_H6W10");
    let right_h6 = stitched_neighbors(&db, &arch, &wires, &RouteNode::new(4, 53, right_h6_wire));
    assert!(
        right_h6
            .iter()
            .any(|&(x, y, wire)| x == 4 && y == 52 && wires.resolve(wire) == "H6D10")
    );

    let left_short_wire = wires.intern("LEFT_E13");
    let left_short = stitched_neighbors(&db, &arch, &wires, &RouteNode::new(5, 1, left_short_wire));
    assert!(
        left_short
            .iter()
            .any(|&(x, y, wire)| x == 5 && y == 2 && wires.resolve(wire) == "W13")
    );

    let left_h6_wire = wires.intern("LEFT_H6E3");
    let left_h6 = stitched_neighbors(&db, &arch, &wires, &RouteNode::new(5, 1, left_h6_wire));
    assert!(
        left_h6
            .iter()
            .any(|&(x, y, wire)| x == 5 && y == 2 && wires.resolve(wire) == "H6A3")
    );
}

#[test]
fn tile_side_neighbor_directions_are_consistent() {
    let left = super::neighbor_for_port(
        3,
        4,
        super::TilePortRef {
            side: TileSide::Left,
            index: 7,
        },
    );
    let right = super::neighbor_for_port(
        3,
        4,
        super::TilePortRef {
            side: TileSide::Right,
            index: 7,
        },
    );
    let top = super::neighbor_for_port(
        3,
        4,
        super::TilePortRef {
            side: TileSide::Top,
            index: 7,
        },
    );
    let bottom = super::neighbor_for_port(
        3,
        4,
        super::TilePortRef {
            side: TileSide::Bottom,
            index: 7,
        },
    );

    assert_eq!(left, Some((3, 3, TileSide::Right)));
    assert_eq!(right, Some((3, 5, TileSide::Left)));
    assert_eq!(top, Some((2, 4, TileSide::Bottom)));
    assert_eq!(bottom, Some((4, 4, TileSide::Top)));
}

#[test]
fn real_arch_components_capture_multi_tile_h6_and_v6_spans() {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&std::path::PathBuf::from(
        env!("CARGO_MANIFEST_DIR"),
    ))
    .ok() else {
        return;
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    if !arch_path.exists() {
        return;
    }

    let arch = load_arch(&arch_path).expect("load arch");
    let mut wires = WireInterner::default();
    let db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");
    let components = build_stitched_components(&db, &arch, &wires);

    let h6_bounds = components
        .bounds(&RouteNode::new(16, 11, wires.intern("H6W6")))
        .expect("H6W6 component");
    assert_eq!(h6_bounds.min_x, 16);
    assert_eq!(h6_bounds.max_x, 16);
    assert!(h6_bounds.min_y <= 5);
    assert!(h6_bounds.max_y >= 11);

    let v6_bounds = components
        .bounds(&RouteNode::new(16, 5, wires.intern("V6N7")))
        .expect("V6N7 component");
    assert!(v6_bounds.min_x < 16);
    assert_eq!(v6_bounds.max_x, 16);
    assert_eq!(v6_bounds.min_y, 5);
    assert_eq!(v6_bounds.max_y, 5);
}

#[test]
fn clock_spine_stitching_reaches_center_and_clkv_tiles() {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&std::path::PathBuf::from(
        env!("CARGO_MANIFEST_DIR"),
    ))
    .ok() else {
        return;
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    if !arch_path.exists() {
        return;
    }

    let arch = load_arch(&arch_path).expect("load arch");
    let mut wires = WireInterner::default();
    let _db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");

    let from_clkb = RouteNode::new(34, 27, wires.intern("CLKB_GCLK0"));
    let clkb_neighbors = clock_spine_neighbors(&arch, &wires, &from_clkb);
    assert!(
        clkb_neighbors
            .iter()
            .any(|&(x, y, wire)| x == 17 && y == 27 && wires.resolve(wire) == "CLKC_GCLK0")
    );

    let db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");
    let from_clkc = RouteNode::new(17, 27, wires.intern("CLKC_VGCLK0"));
    let clkc_neighbors = stitched_neighbors(&db, &arch, &wires, &from_clkc);
    assert!(
        clkc_neighbors
            .iter()
            .any(|&(x, y, wire)| x == 16 && y == 27 && wires.resolve(wire) == "CLKV_VGCLK0")
    );
}

#[test]
fn clock_spine_fanout_respects_left_right_halves() {
    let Some(bundle) = crate::resource::ResourceBundle::discover_from(&std::path::PathBuf::from(
        env!("CARGO_MANIFEST_DIR"),
    ))
    .ok() else {
        return;
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    if !arch_path.exists() {
        return;
    }

    let arch = load_arch(&arch_path).expect("load arch");
    let mut wires = WireInterner::default();
    let _db = load_tile_stitch_db(&arch_path, &mut wires).expect("load stitch db");

    let from_left = RouteNode::new(16, 27, wires.intern("CLKV_GCLK_BUFL0"));
    let left_neighbors = clock_spine_neighbors(&arch, &wires, &from_left);
    assert!(
        left_neighbors
            .iter()
            .any(|&(x, y, wire)| { x == 16 && y == 26 && wires.resolve(wire) == "GCLK0" })
    );
    assert!(
        !left_neighbors
            .iter()
            .any(|&(x, y, wire)| { x == 16 && y == 28 && wires.resolve(wire) == "GCLK0" })
    );

    let from_right = RouteNode::new(16, 27, wires.intern("CLKV_GCLK_BUFR0"));
    let right_neighbors = clock_spine_neighbors(&arch, &wires, &from_right);
    assert!(
        right_neighbors
            .iter()
            .any(|&(x, y, wire)| { x == 16 && y == 28 && wires.resolve(wire) == "GCLK0" })
    );
    assert!(
        !right_neighbors
            .iter()
            .any(|&(x, y, wire)| { x == 16 && y == 26 && wires.resolve(wire) == "GCLK0" })
    );
}
