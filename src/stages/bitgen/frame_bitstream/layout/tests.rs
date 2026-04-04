use super::build_tile_columns;
use crate::{
    bitgen::{ConfigImage, TileBitAssignment, TileConfigImage},
    cil::parse_cil_str,
    resource::{Arch, TileInstance},
    route::RouteBit,
};
use rustc_hash::FxHashMap as HashMap;
use std::collections::BTreeMap;

#[test]
fn applies_default_transmission_bits_into_frame_images() {
    let cil = parse_cil_str(
        r##"
            <device name="mini">
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

    let arch = Arch {
        width: 1,
        height: 1,
        tiles: BTreeMap::from([(
            (0, 0),
            TileInstance {
                name: "T0".to_string(),
                tile_type: "CENTER".to_string(),
                logic_x: 0,
                logic_y: 0,
                bit_x: 0,
                bit_y: 0,
                phy_x: 0,
                phy_y: 0,
            },
        )]),
        ..Arch::default()
    };
    let transmission_defaults = [(
        "GSB_CNT".to_string(),
        vec![RouteBit {
            basic_cell: "sw0".to_string(),
            sram_name: "EN".to_string(),
            value: 0,
        }],
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let mut notes = Vec::new();
    let columns = build_tile_columns(
        &arch,
        &cil,
        &ConfigImage::default(),
        &transmission_defaults,
        &mut notes,
    );
    let tile = &columns
        .get(&0)
        .expect("column 0")
        .first()
        .expect("tile image");

    assert_eq!(tile.bits, vec![0]);
    assert!(notes.is_empty());
}

#[test]
fn relocates_default_site_bits_into_owner_tiles() {
    let cil = parse_cil_str(
        r##"
            <device name="mini">
              <site_library>
                <block_site name="SRC">
                  <config_info amount="1">
                    <cfg_element name="MODE">
                      <function name="ON" default="yes">
                        <sram basic_cell="CFG" name="BIT" content="0"/>
                      </function>
                    </cfg_element>
                  </config_info>
                </block_site>
              </site_library>
              <cluster_library>
                <homogeneous_cluster name="SRC1x1" type="SRC"/>
              </cluster_library>
              <tile_library>
                <tile name="OWNER" sram_amount="R1C1"/>
                <tile name="SOURCE" sram_amount="R1C1">
                  <cluster_info amount="1">
                    <cluster type="SRC1x1">
                      <site name="SRC0" position="R0C0">
                        <site_sram>
                          <sram basic_cell="CFG" sram_name="BIT" local_place="B0W0" owner_tile="OWNER" brick_offset="R0C-1"/>
                        </site_sram>
                      </site>
                    </cluster>
                  </cluster_info>
                </tile>
              </tile_library>
            </device>
            "##,
    )
    .expect("parse mini cil");

    let arch = Arch {
        width: 1,
        height: 2,
        tiles: BTreeMap::from([
            (
                (0, 0),
                TileInstance {
                    name: "OWN0".to_string(),
                    tile_type: "OWNER".to_string(),
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
                    name: "SRC0".to_string(),
                    tile_type: "SOURCE".to_string(),
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

    let mut notes = Vec::new();
    let columns = build_tile_columns(
        &arch,
        &cil,
        &ConfigImage::default(),
        &HashMap::default(),
        &mut notes,
    );

    let owner = columns
        .get(&0)
        .expect("owner column")
        .first()
        .expect("owner tile");
    let source = columns
        .get(&1)
        .expect("source column")
        .first()
        .expect("source tile");

    assert_eq!(owner.bits, vec![0]);
    assert_eq!(source.bits, vec![1]);
    assert!(notes.is_empty());
}

#[test]
fn relocates_config_assignments_into_owner_tiles() {
    let cil = parse_cil_str(
        r##"
            <device name="mini">
              <site_library>
                <block_site name="SRC">
                  <config_info amount="1">
                    <cfg_element name="MODE">
                      <function name="ON">
                        <sram basic_cell="CFG" name="BIT" content="0"/>
                      </function>
                    </cfg_element>
                  </config_info>
                </block_site>
              </site_library>
              <cluster_library>
                <homogeneous_cluster name="SRC1x1" type="SRC"/>
              </cluster_library>
              <tile_library>
                <tile name="OWNER" sram_amount="R1C1"/>
                <tile name="SOURCE" sram_amount="R1C1">
                  <cluster_info amount="1">
                    <cluster type="SRC1x1">
                      <site name="SRC0" position="R0C0">
                        <site_sram>
                          <sram basic_cell="CFG" sram_name="BIT" local_place="B0W0" owner_tile="OWNER" brick_offset="R0C-1"/>
                        </site_sram>
                      </site>
                    </cluster>
                  </cluster_info>
                </tile>
              </tile_library>
            </device>
            "##,
    )
    .expect("parse mini cil");

    let arch = Arch {
        width: 1,
        height: 2,
        tiles: BTreeMap::from([
            (
                (0, 0),
                TileInstance {
                    name: "OWN0".to_string(),
                    tile_type: "OWNER".to_string(),
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
                    name: "SRC0".to_string(),
                    tile_type: "SOURCE".to_string(),
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
    let config_image = ConfigImage {
        tiles: vec![TileConfigImage {
            tile_name: "SRC0".to_string(),
            tile_type: "SOURCE".to_string(),
            x: 0,
            y: 1,
            rows: 1,
            cols: 1,
            configs: Vec::new(),
            assignments: vec![TileBitAssignment {
                site_name: "SRC0".to_string(),
                cfg_name: "MODE".to_string(),
                function_name: "ON".to_string(),
                basic_cell: "CFG".to_string(),
                sram_name: "BIT".to_string(),
                row: 0,
                col: 0,
                value: 0,
            }],
        }],
        notes: Vec::new(),
    };

    let mut notes = Vec::new();
    let columns = build_tile_columns(&arch, &cil, &config_image, &HashMap::default(), &mut notes);

    let owner = columns
        .get(&0)
        .expect("owner column")
        .first()
        .expect("owner tile");
    let source = columns
        .get(&1)
        .expect("source column")
        .first()
        .expect("source tile");

    assert_eq!(owner.bits, vec![0]);
    assert_eq!(source.bits, vec![1]);
    assert!(notes.is_empty());
}
