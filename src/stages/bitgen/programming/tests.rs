use super::{
    build_programming_image,
    derive::derive_site_programs,
    types::{
        BlockRamProgram, RequestedConfig, SiteProgramKind, SliceClockEnableMode, SliceFfDataPath,
        SliceLutOutputUsage, SliceSetResetMode,
    },
};
use crate::{
    bitgen::DeviceDesignIndex,
    bitgen::{DeviceCell, DeviceDesign, DeviceEndpoint, DeviceNet},
    build_config_image,
    cil::load_cil,
    cil::parse_cil_str,
    domain::{CellKind, NetOrigin, SiteKind},
    io::load_design,
    ir::Property,
    load_arch, lower_design,
};
use std::path::PathBuf;

fn logic_slice_program(device: &DeviceDesign) -> super::types::SliceProgram {
    let index = DeviceDesignIndex::build(device);
    derive_site_programs(device, &index)
        .into_iter()
        .find_map(|site| match site.kind {
            SiteProgramKind::LogicSlice(program) => Some(program),
            SiteProgramKind::BlockRam(_)
            | SiteProgramKind::Iob(_)
            | SiteProgramKind::Gclk
            | SiteProgramKind::GclkIob => None,
        })
        .expect("logic slice program")
}

fn compiled_logic_slice_requests(device: &DeviceDesign, cil_xml: &str) -> Vec<RequestedConfig> {
    let cil = parse_cil_str(cil_xml).expect("parse mini cil");
    build_programming_image(device, &cil, None)
        .sites
        .into_iter()
        .find(|site| site.site_kind == SiteKind::LogicSlice)
        .expect("compiled logic slice site")
        .requests
}

fn block_ram_program(device: &DeviceDesign) -> BlockRamProgram {
    let index = DeviceDesignIndex::build(device);
    derive_site_programs(device, &index)
        .into_iter()
        .find_map(|site| match site.kind {
            SiteProgramKind::BlockRam(program) => Some(program),
            SiteProgramKind::LogicSlice(_)
            | SiteProgramKind::Iob(_)
            | SiteProgramKind::Gclk
            | SiteProgramKind::GclkIob => None,
        })
        .expect("block ram program")
}

fn compiled_block_ram_requests(device: &DeviceDesign, cil_xml: &str) -> Vec<RequestedConfig> {
    let cil = parse_cil_str(cil_xml).expect("parse mini cil");
    build_programming_image(device, &cil, None)
        .sites
        .into_iter()
        .find(|site| site.site_kind == SiteKind::BlockRam)
        .expect("compiled block ram site")
        .requests
}

fn mini_logic_slice_lut_cil() -> &'static str {
    r##"
        <device name="mini">
          <site_library>
            <block_site name="SLICE">
              <config_info amount="1">
                <cfg_element name="F">
                  <function name="0xFFFF" quomodo="srambit" manner="computation" default="no">
                    <sram basic_cell="FLUT0" name="SRAM" address="0"/>
                    <sram basic_cell="FLUT1" name="SRAM" address="1"/>
                    <sram basic_cell="FLUT2" name="SRAM" address="2"/>
                    <sram basic_cell="FLUT3" name="SRAM" address="3"/>
                    <sram basic_cell="FLUT4" name="SRAM" address="4"/>
                    <sram basic_cell="FLUT5" name="SRAM" address="5"/>
                    <sram basic_cell="FLUT6" name="SRAM" address="6"/>
                    <sram basic_cell="FLUT7" name="SRAM" address="7"/>
                    <sram basic_cell="FLUT8" name="SRAM" address="8"/>
                    <sram basic_cell="FLUT9" name="SRAM" address="9"/>
                    <sram basic_cell="FLUT10" name="SRAM" address="10"/>
                    <sram basic_cell="FLUT11" name="SRAM" address="11"/>
                    <sram basic_cell="FLUT12" name="SRAM" address="12"/>
                    <sram basic_cell="FLUT13" name="SRAM" address="13"/>
                    <sram basic_cell="FLUT14" name="SRAM" address="14"/>
                    <sram basic_cell="FLUT15" name="SRAM" address="15"/>
                  </function>
                </cfg_element>
              </config_info>
            </block_site>
          </site_library>
        </device>
        "##
}

fn mini_block_ram_cil() -> &'static str {
    r##"
        <device name="mini">
          <site_library>
            <block_site name="BRAM">
              <config_info amount="10">
                <cfg_element name="WEAMUX">
                  <function name="WEA" default="no"/>
                </cfg_element>
                <cfg_element name="WEBMUX">
                  <function name="WEB" default="no"/>
                </cfg_element>
                <cfg_element name="ENAMUX">
                  <function name="ENA" default="no"/>
                </cfg_element>
                <cfg_element name="ENBMUX">
                  <function name="ENB" default="no"/>
                </cfg_element>
                <cfg_element name="RSTAMUX">
                  <function name="RSTA" default="no"/>
                </cfg_element>
                <cfg_element name="RSTBMUX">
                  <function name="RSTB" default="no"/>
                </cfg_element>
                <cfg_element name="CLKAMUX">
                  <function name="CLK" default="no"/>
                </cfg_element>
                <cfg_element name="CLKBMUX">
                  <function name="CLK" default="no"/>
                </cfg_element>
                <cfg_element name="PORTA_ATTR">
                  <function name="4096X1" default="no"/>
                  <function name="2048X2" default="no"/>
                  <function name="256X16" default="yes"/>
                </cfg_element>
                <cfg_element name="PORTB_ATTR">
                  <function name="4096X1" default="no"/>
                  <function name="256X16" default="yes"/>
                </cfg_element>
              </config_info>
            </block_site>
          </site_library>
        </device>
        "##
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[ignore = "debug helper for physical XML bitgen request tracing"]
fn debug_actual_routed_slice_requests() {
    let root = repo_root();
    let design =
        load_design(&root.join("build/bram_cpp_yosys/04-routed.xml")).expect("load design");
    let arch = load_arch(&root.join("resources/hw_lib/fdp3p7_arch.xml")).expect("load arch");
    let cil = load_cil(&root.join("resources/hw_lib/fdp3p7_cil.xml")).expect("load cil");
    let lowered = lower_design(design, &arch, Some(&cil), &[]).expect("lower design");
    let programmed = build_programming_image(&lowered, &cil, None);
    let image = build_config_image(&lowered, &cil, Some(&arch), None).expect("config image");

    for site in programmed.sites.iter().filter(|site| {
        site.tile_name == "R24C1" || site.tile_name == "R2C32" || site.tile_name == "R31C3"
    }) {
        println!(
            "site {} {} {} {}",
            site.tile_name, site.site_name, site.x, site.y
        );
        for request in &site.requests {
            println!("  {}={}", request.cfg_name, request.function_name);
        }
    }
    for tile in image.tiles.iter().filter(|tile| {
        tile.tile_name == "R24C1" || tile.tile_name == "R2C32" || tile.tile_name == "R31C3"
    }) {
        println!("tile {} {} {}", tile.tile_name, tile.x, tile.y);
        for cfg in &tile.configs {
            println!(
                "  cfg {} {}={}",
                cfg.site_name, cfg.cfg_name, cfg.function_name
            );
        }
        for bit in tile
            .assignments
            .iter()
            .filter(|bit| bit.cfg_name == "SRMUX" || bit.cfg_name == "SRFFMUX")
        {
            println!(
                "  bit {} {}={} -> B{}W{}",
                bit.site_name, bit.cfg_name, bit.function_name, bit.row, bit.col
            );
        }
    }
}

#[test]
fn detects_local_lut_ff_data_path_for_paired_driver() {
    let lut0 = DeviceCell::new("lut0", CellKind::Lut, "LUT4")
        .with_properties(vec![Property::new("lut_init", "0xA")])
        .placed(
            SiteKind::LogicSlice,
            "S0",
            "LUT0",
            "T0",
            "CENTER",
            (0, 0, 0),
        );
    let ff0 = DeviceCell::new("ff0", CellKind::Ff, "DFFHQ").placed(
        SiteKind::LogicSlice,
        "S0",
        "FF0",
        "T0",
        "CENTER",
        (0, 0, 0),
    );
    let device = DeviceDesign {
        cells: vec![lut0, ff0],
        nets: vec![
            DeviceNet::new("n0", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut0", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ff0", "D", (0, 0, 0))),
        ],
        ..DeviceDesign::default()
    };

    let slice = logic_slice_program(&device);
    assert_eq!(
        slice.slots[0].ff.as_ref().expect("slot0 ff").data_path,
        SliceFfDataPath::LocalLut
    );
}

#[test]
fn detects_site_bypass_for_nonlocal_ff_driver() {
    let ff0 = DeviceCell::new("ff0", CellKind::Ff, "DFFHQ").placed(
        SiteKind::LogicSlice,
        "S0",
        "FF0",
        "T0",
        "CENTER",
        (0, 0, 0),
    );
    let other = DeviceCell::new("lut1", CellKind::Lut, "LUT4")
        .with_properties(vec![Property::new("lut_init", "0xA")])
        .placed(
            SiteKind::LogicSlice,
            "S1",
            "LUT1",
            "T0",
            "CENTER",
            (0, 0, 1),
        );
    let device = DeviceDesign {
        cells: vec![ff0, other],
        nets: vec![
            DeviceNet::new("n0", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut1", "O", (0, 0, 1)))
                .with_sink(DeviceEndpoint::cell("ff0", "D", (0, 0, 0))),
        ],
        ..DeviceDesign::default()
    };

    let slice = logic_slice_program(&device);
    assert_eq!(
        slice.slots[0].ff.as_ref().expect("slot0 ff").data_path,
        SliceFfDataPath::SiteBypass
    );
}

#[test]
fn classifies_hidden_vs_routed_lut_outputs_before_encoding() {
    let lut0 = DeviceCell::new("lut0", CellKind::Lut, "LUT4")
        .with_properties(vec![Property::new("lut_init", "0xA")])
        .placed(
            SiteKind::LogicSlice,
            "S0",
            "LUT0",
            "T0",
            "CENTER",
            (0, 0, 0),
        );
    let ff0 = DeviceCell::new("ff0", CellKind::Ff, "DFFHQ").placed(
        SiteKind::LogicSlice,
        "S0",
        "FF0",
        "T0",
        "CENTER",
        (0, 0, 0),
    );
    let hidden_only = DeviceDesign {
        cells: vec![lut0.clone(), ff0.clone()],
        nets: vec![
            DeviceNet::new("n0", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut0", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ff0", "D", (0, 0, 0))),
        ],
        ..DeviceDesign::default()
    };
    let routed = DeviceDesign {
        cells: vec![lut0, ff0],
        nets: vec![
            DeviceNet::new("n0", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut0", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ff0", "D", (0, 0, 0))),
            DeviceNet::new("n1", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut0", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::port("out", "OUT", (1, 1, 0))),
        ],
        ..DeviceDesign::default()
    };

    assert_eq!(
        logic_slice_program(&hidden_only).slots[0]
            .lut
            .as_ref()
            .expect("slot0 lut")
            .output_usage,
        SliceLutOutputUsage::HiddenLocalOnly
    );
    assert_eq!(
        logic_slice_program(&routed).slots[0]
            .lut
            .as_ref()
            .expect("slot0 lut")
            .output_usage,
        SliceLutOutputUsage::RoutedOutput
    );
}

#[test]
fn detects_clock_enable_usage_in_site_program() {
    let ff0 = DeviceCell::new("ff0", CellKind::Ff, "DFFHQ").placed(
        SiteKind::LogicSlice,
        "S0",
        "FF0",
        "T0",
        "CENTER",
        (0, 0, 0),
    );
    let device = DeviceDesign {
        cells: vec![ff0],
        nets: vec![
            DeviceNet::new("ce", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("ce_in", "IN", (1, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ff0", "CE", (0, 0, 0))),
        ],
        ..DeviceDesign::default()
    };

    let slice = logic_slice_program(&device);
    assert_eq!(slice.clock_enable_mode, SliceClockEnableMode::DirectCe);
}

#[test]
fn detects_shared_active_low_set_reset_usage_in_site_program() {
    let ff0 = DeviceCell::new("ff0", CellKind::Ff, "DFFHQ").placed(
        SiteKind::LogicSlice,
        "S0",
        "FF0",
        "T0",
        "CENTER",
        (0, 0, 0),
    );
    let device = DeviceDesign {
        cells: vec![ff0],
        nets: vec![
            DeviceNet::new("rst", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("rst_in", "IN", (1, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ff0", "RN", (0, 0, 0))),
        ],
        ..DeviceDesign::default()
    };

    let slice = logic_slice_program(&device);
    assert_eq!(slice.set_reset_mode, SliceSetResetMode::ActiveLowShared);

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "SRMUX" && request.function_name == "SR_B")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "SRFFMUX" && request.function_name == "0")
    );
}

#[test]
fn preserves_high_ff_init_from_imported_cell_properties() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("ff0", CellKind::Ff, "DFFHQ")
                .with_properties(vec![Property::new("init", "1")])
                .placed(SiteKind::LogicSlice, "S0", "FF0", "T0", "CENTER", (0, 0, 0)),
        ],
        ..DeviceDesign::default()
    };

    let slice = logic_slice_program(&device);
    assert_eq!(
        slice.slots[0].ff.as_ref().expect("slot0 ff").init,
        crate::domain::SequentialInitValue::High
    );

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "INITX" && request.function_name == "HIGH")
    );
}

#[test]
fn widens_small_lut_init_to_site_truth_table_width_before_encoding() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("lut0", CellKind::Lut, "LUT2")
                .with_properties(vec![Property::new("lut_init", "1")])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT0",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
        ],
        ..DeviceDesign::default()
    };

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());

    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "F" && request.function_name == "0x1111")
    );
}

#[test]
fn prefers_raw_init_over_canonical_lut_init_when_both_are_present() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("lut0", CellKind::Lut, "LUT2")
                .with_properties(vec![
                    Property::new("init", "12"),
                    Property::new("lut_init", "0xC"),
                ])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT0",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
        ],
        ..DeviceDesign::default()
    };

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());

    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "F" && request.function_name == "0x1212")
    );
}

#[test]
fn programming_boundary_accepts_raw_init_without_canonical_lut_init() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("lut0", CellKind::Lut, "LUT2")
                .with_properties(vec![Property::new("init", "12")])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT0",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
        ],
        ..DeviceDesign::default()
    };

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());

    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "F" && request.function_name == "0x1212")
    );
}

#[test]
fn normalized_lut1_prefers_raw_decimal_init_compact_semantics() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("lut0", CellKind::Lut, "LUT1")
                .with_properties(vec![
                    Property::new("init", "15"),
                    Property::new("lut_init", "0x3"),
                    Property::new("pin_map_ADR0", "0,1"),
                ])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT0",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
        ],
        ..DeviceDesign::default()
    };

    let requests = compiled_logic_slice_requests(&device, mini_logic_slice_lut_cil());

    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "F" && request.function_name == "0x1515")
    );
}

#[test]
fn routed_lut_only_slice_emits_usage_bits_without_ff_controls() {
    let device = DeviceDesign {
        cells: vec![
            DeviceCell::new("lut0", CellKind::Lut, "LUT2")
                .with_properties(vec![Property::new("lut_init", "0x5")])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT0",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
            DeviceCell::new("lut1", CellKind::Lut, "LUT2")
                .with_properties(vec![Property::new("lut_init", "0xA")])
                .placed(
                    SiteKind::LogicSlice,
                    "S0",
                    "LUT1",
                    "T0",
                    "CENTER",
                    (0, 0, 0),
                ),
        ],
        nets: vec![
            DeviceNet::new("n0", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut0", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::port("out0", "OUT", (1, 0, 0))),
            DeviceNet::new("n1", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::cell("lut1", "O", (0, 0, 0)))
                .with_sink(DeviceEndpoint::port("out1", "OUT", (1, 1, 0))),
        ],
        ..DeviceDesign::default()
    };

    let requests = compiled_logic_slice_requests(
        &device,
        r##"
        <device name="mini">
          <site_library>
            <block_site name="SLICE">
              <config_info amount="2">
                <cfg_element name="F">
                  <function name="0x0" quomodo="srambit" manner="computation" default="no">
                    <sram basic_cell="FLUT0" name="SRAM" address="0"/>
                    <sram basic_cell="FLUT1" name="SRAM" address="1"/>
                    <sram basic_cell="FLUT2" name="SRAM" address="2"/>
                    <sram basic_cell="FLUT3" name="SRAM" address="3"/>
                  </function>
                </cfg_element>
                <cfg_element name="G">
                  <function name="0x0" quomodo="srambit" manner="computation" default="no">
                    <sram basic_cell="GLUT0" name="SRAM" address="0"/>
                    <sram basic_cell="GLUT1" name="SRAM" address="1"/>
                    <sram basic_cell="GLUT2" name="SRAM" address="2"/>
                    <sram basic_cell="GLUT3" name="SRAM" address="3"/>
                  </function>
                </cfg_element>
              </config_info>
            </block_site>
          </site_library>
        </device>
        "##,
    );

    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "XUSED" && request.function_name == "0")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "YUSED" && request.function_name == "0")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "FXMUX" && request.function_name == "F")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "GYMUX" && request.function_name == "G")
    );
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request.cfg_name.as_str(), "DXMUX" | "DYMUX" | "CKINV"))
    );
}

#[test]
fn single_port_block_ram_program_maps_port_attr_controls_and_init_words() {
    let ram = DeviceCell::new("ram0", CellKind::BlockRam, "BLOCKRAM_1")
        .with_properties(vec![
            Property::new("PORT_ATTR", "2048X2"),
            Property::new("INIT_00", "0123456789ABCDEF"),
        ])
        .placed(
            SiteKind::BlockRam,
            "BRAM",
            "BRAM",
            "BRAM0",
            "LBRAMD",
            (4, 5, 0),
        );
    let device = DeviceDesign {
        cells: vec![ram],
        nets: vec![
            DeviceNet::new("clk", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("clk", "IN", (8, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "CLK", (4, 5, 0))),
            DeviceNet::new("en", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("en", "IN", (8, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "EN", (4, 5, 0))),
            DeviceNet::new("we", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("we", "IN", (8, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "WE", (4, 5, 0))),
        ],
        ..DeviceDesign::default()
    };

    let program = block_ram_program(&device);
    assert_eq!(program.port_a_attr.as_deref(), Some("2048X2"));
    assert_eq!(program.port_b_attr.as_deref(), None);
    assert!(program.clka_used);
    assert!(program.ena_used);
    assert!(program.wea_used);
    assert!(!program.rsta_used);
    assert_eq!(
        program
            .init_words
            .iter()
            .find(|(name, _)| name == "INIT_00")
            .map(|(_, value)| value.as_str()),
        Some("0123456789ABCDEF")
    );

    let requests = compiled_block_ram_requests(&device, mini_block_ram_cil());
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "PORTA_ATTR" && request.function_name == "2048X2")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "CLKAMUX" && request.function_name == "CLK")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "ENAMUX" && request.function_name == "ENA")
    );
    assert!(
        requests
            .iter()
            .any(|request| request.cfg_name == "WEAMUX" && request.function_name == "WEA")
    );
    assert!(!requests.iter().any(|request| {
        matches!(
            request.cfg_name.as_str(),
            "PORTB_ATTR" | "CLKBMUX" | "ENBMUX" | "WEBMUX" | "INIT_00"
        )
    }));
}

#[test]
fn block_ram_program_preserves_imported_lowercase_init_properties() {
    let ram = DeviceCell::new("ram0", CellKind::BlockRam, "BLOCKRAM_1")
        .with_properties(vec![
            Property::new(
                "init_00",
                "256'h00000000000000000000000000000000000000000000000000000000cafebabe",
            ),
            Property::new(
                "init_01",
                "256'h00000000000000000000000000000000000000000000000000000000deadbeef",
            ),
            Property::new("port_attr", "4096X1"),
        ])
        .placed(
            SiteKind::BlockRam,
            "BRAM",
            "BRAM",
            "BRAM0",
            "LBRAMD",
            (4, 0, 0),
        );
    let device = DeviceDesign {
        cells: vec![ram],
        ..DeviceDesign::default()
    };

    let program = block_ram_program(&device);
    assert_eq!(program.port_a_attr.as_deref(), Some("4096X1"));
    assert!(
        program
            .init_words
            .iter()
            .any(|(name, value)| name == "INIT_00" && value.ends_with("cafebabe"))
    );
    assert!(
        program
            .init_words
            .iter()
            .any(|(name, value)| name == "INIT_01" && value.ends_with("deadbeef"))
    );
}

#[test]
fn dual_port_block_ram_program_emits_cpp_compatible_dual_port_requests() {
    let ram = DeviceCell::new("ram0", CellKind::BlockRam, "BLOCKRAM_2")
        .with_properties(vec![
            Property::new("PORTA_ATTR", "4096X1"),
            Property::new("PORTB_ATTR", "256X16"),
            Property::new("INIT_00", "FEDCBA9876543210"),
        ])
        .placed(
            SiteKind::BlockRam,
            "BRAM",
            "BRAM",
            "BRAM1",
            "RBRAMD",
            (10, 2, 0),
        );
    let device = DeviceDesign {
        cells: vec![ram],
        nets: vec![
            DeviceNet::new("clka", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("clka", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "CLKA", (10, 2, 0))),
            DeviceNet::new("ena", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("ena", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "ENA", (10, 2, 0))),
            DeviceNet::new("rsta", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("rsta", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "RSTA", (10, 2, 0))),
            DeviceNet::new("clkb", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("clkb", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "CLKB", (10, 2, 0))),
            DeviceNet::new("web", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("web", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "WEB", (10, 2, 0))),
            DeviceNet::new("enb", NetOrigin::Logical)
                .with_driver(DeviceEndpoint::port("enb", "IN", (0, 0, 0)))
                .with_sink(DeviceEndpoint::cell("ram0", "ENB", (10, 2, 0))),
        ],
        ..DeviceDesign::default()
    };

    let program = block_ram_program(&device);
    assert_eq!(program.port_a_attr.as_deref(), Some("4096X1"));
    assert_eq!(program.port_b_attr.as_deref(), Some("256X16"));
    assert!(program.clka_used);
    assert!(program.ena_used);
    assert!(program.rsta_used);
    assert!(program.clkb_used);
    assert!(program.enb_used);
    assert!(program.web_used);
    assert!(!program.wea_used);
    assert!(!program.rstb_used);

    let requests = compiled_block_ram_requests(&device, mini_block_ram_cil());
    for (cfg_name, function_name) in [
        ("PORTA_ATTR", "4096X1"),
        ("PORTB_ATTR", "256X16"),
        ("CLKAMUX", "CLK"),
        ("ENAMUX", "ENA"),
        ("RSTAMUX", "RSTA"),
        ("CLKBMUX", "CLK"),
        ("ENBMUX", "ENB"),
        ("WEBMUX", "WEB"),
    ] {
        assert!(requests.iter().any(|request| {
            request.cfg_name == cfg_name && request.function_name == function_name
        }));
    }
    assert!(
        !requests.iter().any(|request| {
            matches!(request.cfg_name.as_str(), "WEAMUX" | "RSTBMUX" | "INIT_00")
        })
    );
}
