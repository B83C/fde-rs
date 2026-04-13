use rustc_hash::FxHashMap as HashMap;
use std::{collections::BTreeMap, fs};

use tempfile::NamedTempFile;

use super::{
    decode::decode_text_bitstream, encode::build_major_payloads, layout::build_tile_columns,
    serialize_text_bitstream,
};
use crate::{
    bitgen::{ConfigImage, TileBitAssignment, TileConfigImage},
    build_config_image,
    cil::{load_cil, parse_cil_str},
    io::load_design,
    load_arch, lower_design,
    resource::{Arch, TileInstance, routing},
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

#[test]
#[ignore = "debug helper for actual frame bitstream mismatch tracing"]
fn debug_actual_bram_frame_columns() {
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn tile_bit(tile: &super::model::TileFrameImage, row: usize, col: usize) -> u8 {
        tile.bits[row * tile.cols + col]
    }

    fn find_tile<'a>(
        columns: &'a std::collections::BTreeMap<usize, Vec<super::model::TileFrameImage>>,
        bit_y: usize,
        bit_x: usize,
        tile_name: &str,
    ) -> &'a super::model::TileFrameImage {
        columns[&bit_y]
            .iter()
            .find(|tile| tile.tile_name == tile_name && tile.bit_x == bit_x)
            .expect("tile present")
    }

    let root = repo_root();
    let arch = load_arch(&root.join("resources/hw_lib/fdp3p7_arch.xml")).expect("load arch");
    let cil = load_cil(&root.join("resources/hw_lib/fdp3p7_cil.xml")).expect("load cil");
    let design =
        load_design(&root.join("build/bram_cpp_yosys/04-routed.xml")).expect("load routed");
    let arch_path = root.join("resources/hw_lib/fdp3p7_arch.xml");
    let lowered = lower_design(design, &arch, Some(&cil), &[]).expect("lower routed");
    let config_image = build_config_image(&lowered, &cil, Some(&arch), None).expect("config image");
    let transmission_defaults =
        routing::load_site_route_defaults(&arch_path, &cil).expect("load transmission defaults");

    let mut notes = Vec::new();
    let layout = build_tile_columns(
        &arch,
        &cil,
        &config_image,
        &transmission_defaults,
        &mut notes,
    );
    assert!(notes.is_empty(), "layout notes: {notes:#?}");

    let cpp_text = fs::read_to_string(root.join("build/bram_cpp_yosys/06-output.bit"))
        .expect("read cpp bitstream");
    let rust_text =
        fs::read_to_string(root.join("build/bram_mix_rust_bitgen_after_constfix/06-output.bit"))
            .expect("read rust bitstream");
    let cpp = decode_text_bitstream(&arch, &cil, &cpp_text).expect("decode cpp bitstream");
    let rust = decode_text_bitstream(&arch, &cil, &rust_text).expect("decode rust bitstream");
    let payloads = build_major_payloads(&cil, &layout).expect("build major payloads");

    let major_payload = |tile_col: usize| {
        payloads
            .iter()
            .find(|payload| {
                cil.majors
                    .iter()
                    .find(|major| major.address == payload.address)
                    .is_some_and(|major| major.tile_col == tile_col)
            })
            .expect("major payload")
    };

    for (tile_name, bit_y, bit_x, bits) in [
        ("R24C1", 2usize, 25usize, &[(4usize, 12usize), (4, 22)][..]),
        (
            "R2C32",
            35usize,
            2usize,
            &[(4usize, 12usize), (4, 22), (4, 25), (4, 35)][..],
        ),
        (
            "R31C3",
            4usize,
            32usize,
            &[(4usize, 12usize), (4, 22), (4, 25), (4, 35)][..],
        ),
    ] {
        let layout_tile = find_tile(&layout, bit_y, bit_x, tile_name);
        let cpp_tile = find_tile(&cpp, bit_y, bit_x, tile_name);
        let rust_tile = find_tile(&rust, bit_y, bit_x, tile_name);

        println!("tile {tile_name}");
        for (row, col) in bits {
            println!(
                "  B{row}W{col}: layout={} cpp={} rust={}",
                tile_bit(layout_tile, *row, *col),
                tile_bit(cpp_tile, *row, *col),
                tile_bit(rust_tile, *row, *col),
            );
        }
    }

    for tile_col in [2usize, 4usize, 35usize] {
        let payload = major_payload(tile_col);
        println!("major tile_col={tile_col} addr={}", payload.address);
        for (word_index, word) in payload.words.iter().enumerate().filter(|(index, _)| {
            matches!(
                (tile_col, *index),
                (2, 254 | 454 | 515 | 715)
                    | (4, 257 | 457 | 513 | 517 | 713 | 717)
                    | (35, 242 | 442 | 502 | 702)
            )
        }) {
            println!("  payload[{word_index}]={word:#010x}");
        }
    }
}
