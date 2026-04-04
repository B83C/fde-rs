use super::{load_route_pips_xml, materialize_route_image};
use crate::{
    cil::{load_cil, parse_cil_str},
    resource::{ResourceBundle, load_arch, routing::build_stitched_components},
    route::RoutedNetPip,
};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::PathBuf,
};
use tempfile::NamedTempFile;

#[test]
fn parses_route_pips_from_routed_xml() {
    let xml = r#"
<design name="toggle">
  <library name="work_lib">
    <module name="toggle" type="GENERIC">
      <contents>
        <net name="q">
          <pip from="S0_XQ" to="OUT2" position="32,9" dir="-&gt;"/>
          <pip from="OUT2" to="LLV0" position="32,9" dir="-&gt;"/>
        </net>
      </contents>
    </module>
  </library>
</design>
"#;

    let pips = load_route_pips_xml(xml).expect("parse pips");

    assert_eq!(
        pips,
        vec![
            RoutedNetPip {
                net_name: "q".to_string(),
                x: 32,
                y: 9,
                from_net: "S0_XQ".to_string(),
                to_net: "OUT2".to_string(),
            },
            RoutedNetPip {
                net_name: "q".to_string(),
                x: 32,
                y: 9,
                from_net: "OUT2".to_string(),
                to_net: "LLV0".to_string(),
            },
        ]
    );
}

#[test]
fn materializes_route_image_from_explicit_pips() {
    let arch_xml = r#"
<architecture name="mini">
  <device_info scale="2,2" slice_per_tile="1" LUT_Inputs="4" />
  <library name="tile">
    <cell name="CENTER" type="TILE">
      <port name="W0" side="left[0:0]"/>
      <port name="W1" side="right[0:0]"/>
    </cell>
  </library>
  <library name="work">
    <instance name="R1C1" libraryRef="tile" cellRef="CENTER" logic_pos="1,1" bit_pos="1,1" phy_pos="1,1"/>
  </library>
</architecture>
"#;
    let cil = parse_cil_str(
        r##"
<device name="mini">
  <element_library>
    <element name="SW2">
      <sram_info amount="1">
        <sram name="EN" default="1"/>
      </sram_info>
      <path_info amount="1">
        <path in="A" out="B">
          <configuration_info amount="1">
            <sram name="EN" content="1"/>
          </configuration_info>
        </path>
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
    let arch_graph_xml = r#"
<architecture>
  <library name="block">
    <cell name="GSB_CNT">
      <contents>
        <instance name="sw0" cellRef="SW2"/>
        <net name="A">
          <portRef instanceRef="sw0" name="A"/>
        </net>
        <net name="B">
          <portRef instanceRef="sw0" name="B"/>
        </net>
      </contents>
    </cell>
  </library>
</architecture>
"#;

    let arch_file = NamedTempFile::new().expect("temp arch");
    fs::write(arch_file.path(), arch_xml).expect("write arch");
    let graph_file = NamedTempFile::new().expect("temp graph");
    fs::write(graph_file.path(), arch_graph_xml).expect("write graph");

    let arch = load_arch(arch_file.path()).expect("load arch");
    let image = materialize_route_image(
        &[RoutedNetPip {
            net_name: "n0".to_string(),
            x: 1,
            y: 1,
            from_net: "A".to_string(),
            to_net: "B".to_string(),
        }],
        &arch,
        graph_file.path(),
        &cil,
    )
    .expect("materialize image");

    assert_eq!(image.notes.len(), 0);
    assert_eq!(image.pips.len(), 1);
    assert_eq!(image.pips[0].tile_name, "R1C1");
    assert_eq!(image.pips[0].tile_type, "CENTER");
    assert_eq!(image.pips[0].site_name, "GSB_CNT");
    assert_eq!(image.pips[0].from_net, "A");
    assert_eq!(image.pips[0].to_net, "B");
    assert_eq!(image.pips[0].bits.len(), 1);
    assert_eq!(image.pips[0].bits[0].basic_cell, "sw0");
    assert_eq!(image.pips[0].bits[0].sram_name, "EN");
    assert_eq!(image.pips[0].bits[0].value, 1);
}

#[test]
fn dedicated_clock_policy_reaches_cxx_clock_spine() {
    let Some(bundle) =
        ResourceBundle::discover_from(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))).ok()
    else {
        return;
    };
    let arch_path = bundle.root.join("fdp3p7_arch.xml");
    let cil_path = bundle.root.join("fdp3p7_cil.xml");
    if !arch_path.exists() || !cil_path.exists() {
        return;
    }

    let arch = load_arch(&arch_path).expect("load arch");
    let cil = load_cil(&cil_path).expect("load cil");
    let mut wires = super::types::WireInterner::default();
    let graphs = crate::resource::routing::load_site_route_graphs(&arch_path, &cil, &mut wires)
        .expect("load graphs");
    let stitch_db = crate::resource::routing::load_tile_stitch_db(&arch_path, &mut wires)
        .expect("load stitch db");
    let stitched_components = build_stitched_components(&stitch_db, &arch, &wires);
    let tile_cache = super::lookup::TileRouteCache::build(&arch, &cil, &graphs);

    let source = super::types::RouteNode::new(34, 27, wires.intern("CLKB_GCLK1_PW"));
    let checkpoints = [
        super::types::RouteNode::new(17, 27, wires.intern("CLKC_GCLK1")),
        super::types::RouteNode::new(17, 27, wires.intern("CLKC_VGCLK1")),
        super::types::RouteNode::new(16, 27, wires.intern("CLKV_VGCLK1")),
        super::types::RouteNode::new(16, 27, wires.intern("CLKV_GCLK_BUFR1")),
        super::types::RouteNode::new(3, 31, wires.intern("S0_CLK_B")),
    ];
    let context = super::router::RouteSinkContext {
        arch: &arch,
        stitched_components: &stitched_components,
        tile_cache: &tile_cache,
        wires: &mut wires,
    };

    let mut frontier = VecDeque::from([source]);
    let mut seen = HashSet::from([source]);
    while let Some(node) = frontier.pop_front() {
        for (neighbor, _) in super::policy::neighbors(
            &context,
            &node,
            super::router::RouteNetKind::DedicatedClock,
            true,
        ) {
            if seen.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }

    for checkpoint in checkpoints {
        assert!(
            seen.contains(&checkpoint),
            "missing C++ dedicated-clock checkpoint {}:{}:{}",
            checkpoint.x,
            checkpoint.y,
            context.wires.resolve(checkpoint.wire),
        );
    }
}
