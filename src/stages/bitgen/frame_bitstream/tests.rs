use rustc_hash::FxHashMap as HashMap;
use std::{collections::BTreeMap, fs};

use tempfile::NamedTempFile;

use super::{decode::decode_text_bitstream, layout::build_tile_columns, serialize_text_bitstream};
use crate::{
    bitgen::{ConfigImage, TileBitAssignment, TileConfigImage},
    cil::parse_cil_str,
    resource::{Arch, TileInstance},
};

#[test]
fn roundtrips_text_bitstream_back_into_tile_columns() {
    let cil = parse_cil_str(
        r##"
        <device name="mini">
          <tile_library>
            <tile name="CENTER" sram_amount="R2C4"/>
          </tile_library>
          <major_library>
            <major address="0" frm_amount="4" tile_col="0"/>
          </major_library>
          <bstrcmd_library>
            <parameter name="bits_per_grp_reversed" value="2"/>
            <parameter name="initialNum" value="1"/>
            <parameter name="FRMLen" value="4"/>
            <parameter name="major_shift" value="17"/>
            <parameter name="mem_amount" value="0"/>
            <parameter name="wrdsAmnt_shift" value="0"/>
            <parameter name="fillblank" value="0"/>
            <command cmd="bsHeader"/>
            <command cmd="adjustSYNC"/>
            <command cmd="insertCMD" parameter="0000_0007, reset CRC"/>
            <command cmd="setFRMLen"/>
            <command cmd="dummy"/>
            <command cmd="writeNomalTiles"/>
          </bstrcmd_library>
        </device>
        "##,
    )
    .expect("parse mini cil");

    let arch = Arch {
        width: 2,
        height: 1,
        tiles: BTreeMap::from([
            (
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
            ),
            (
                (1, 0),
                TileInstance {
                    name: "T1".to_string(),
                    tile_type: "CENTER".to_string(),
                    logic_x: 1,
                    logic_y: 0,
                    bit_x: 1,
                    bit_y: 0,
                    phy_x: 1,
                    phy_y: 0,
                },
            ),
        ]),
        ..Arch::default()
    };
    let config_image = ConfigImage {
        tiles: vec![
            TileConfigImage {
                tile_name: "T0".to_string(),
                tile_type: "CENTER".to_string(),
                x: 0,
                y: 0,
                rows: 2,
                cols: 4,
                configs: Vec::new(),
                assignments: vec![
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "F".to_string(),
                        function_name: "1010".to_string(),
                        basic_cell: "FLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 0,
                        col: 0,
                        value: 0,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "F".to_string(),
                        function_name: "1010".to_string(),
                        basic_cell: "FLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 1,
                        col: 0,
                        value: 1,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "F".to_string(),
                        function_name: "1010".to_string(),
                        basic_cell: "FLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 0,
                        col: 1,
                        value: 1,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "F".to_string(),
                        function_name: "1010".to_string(),
                        basic_cell: "FLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 1,
                        col: 2,
                        value: 0,
                    },
                ],
            },
            TileConfigImage {
                tile_name: "T1".to_string(),
                tile_type: "CENTER".to_string(),
                x: 1,
                y: 0,
                rows: 2,
                cols: 4,
                configs: Vec::new(),
                assignments: vec![
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "G".to_string(),
                        function_name: "0101".to_string(),
                        basic_cell: "GLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 0,
                        col: 0,
                        value: 1,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "G".to_string(),
                        function_name: "0101".to_string(),
                        basic_cell: "GLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 1,
                        col: 1,
                        value: 0,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "G".to_string(),
                        function_name: "0101".to_string(),
                        basic_cell: "GLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 0,
                        col: 2,
                        value: 0,
                    },
                    TileBitAssignment {
                        site_name: "S0".to_string(),
                        cfg_name: "G".to_string(),
                        function_name: "0101".to_string(),
                        basic_cell: "GLUT".to_string(),
                        sram_name: "SRAM".to_string(),
                        row: 1,
                        col: 3,
                        value: 1,
                    },
                ],
            },
        ],
        notes: Vec::new(),
    };

    let mut expected_notes = Vec::new();
    let expected = build_tile_columns(
        &arch,
        &cil,
        &config_image,
        &HashMap::default(),
        &mut expected_notes,
    );
    assert!(expected_notes.is_empty());

    let arch_file = NamedTempFile::new().expect("create temp arch file");
    fs::write(arch_file.path(), "<design name=\"mini\"/>").expect("write temp arch xml");

    let serialized = serialize_text_bitstream("mini", &arch, arch_file.path(), &cil, &config_image)
        .expect("serialize text bitstream")
        .expect("bitstream should be rendered");

    let decoded =
        decode_text_bitstream(&arch, &cil, &serialized.text).expect("decode text bitstream");
    assert_eq!(decoded, expected);
}
