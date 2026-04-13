use std::collections::BTreeMap;

use super::derive::derive_site_programs;
use super::types::{
    BlockRamProgram, IobProgram, LutProgram, ProgrammedMemory, ProgrammedSite, ProgrammingImage,
    RequestedConfig, SequentialProgram, SiteProgram, SiteProgramKind, SliceClockEnableMode,
    SliceFfDataPath, SliceLutOutputUsage, SliceProgram, SliceSetResetMode,
};
use crate::{
    bitgen::DeviceDesign,
    bitgen::{DeviceDesignIndex, literal::address_count},
    cil::{Cil, SiteDef},
    domain::{SliceSequentialConfigKey, SliceSlot},
    route::DeviceRouteImage,
};

pub(crate) fn build_programming_image(
    device: &DeviceDesign,
    cil: &Cil,
    route_image: Option<&DeviceRouteImage>,
) -> ProgrammingImage {
    let mut notes = route_image
        .map(|image| image.notes.clone())
        .unwrap_or_default();
    let index = DeviceDesignIndex::build(device);
    let mut sites = Vec::new();
    let mut memories = Vec::new();

    for site in derive_site_programs(device, &index) {
        if let SiteProgramKind::BlockRam(program) = &site.kind
            && !program.init_words.is_empty()
        {
            memories.push(ProgrammedMemory {
                tile_name: site.tile_name.clone(),
                init_words: program.init_words.clone(),
            });
        }

        let Some(site_def) = cil.site_def(site.site_kind) else {
            notes.push(format!(
                "Missing CIL site definition for {} on tile {}.",
                site.site_kind.as_str(),
                site.tile_name
            ));
            continue;
        };
        let requests = emit_site_requests(&site, site_def);
        if requests.is_empty() {
            continue;
        }
        sites.push(ProgrammedSite {
            tile_name: site.tile_name,
            tile_type: site.tile_type,
            site_kind: site.site_kind,
            site_name: site.site_name,
            x: site.x,
            y: site.y,
            requests,
        });
    }

    ProgrammingImage {
        sites,
        memories,
        routes: route_image
            .map(|image| image.pips.clone())
            .unwrap_or_default(),
        notes,
    }
}

fn emit_site_requests(site: &SiteProgram, site_def: &SiteDef) -> Vec<RequestedConfig> {
    match &site.kind {
        SiteProgramKind::LogicSlice(program) => emit_slice_requests(program, site_def),
        SiteProgramKind::BlockRam(program) => emit_block_ram_requests(program, site_def),
        SiteProgramKind::Iob(program) => emit_iob_requests(program),
        SiteProgramKind::Gclk => vec![
            RequestedConfig {
                cfg_name: "CEMUX".to_string(),
                function_name: "1".to_string(),
            },
            RequestedConfig {
                cfg_name: "DISABLE_ATTR".to_string(),
                function_name: "LOW".to_string(),
            },
        ],
        SiteProgramKind::GclkIob => vec![RequestedConfig {
            cfg_name: "IOATTRBOX".to_string(),
            function_name: "LVTTL".to_string(),
        }],
    }
}

fn emit_slice_requests(program: &SliceProgram, site_def: &SiteDef) -> Vec<RequestedConfig> {
    let mut requests = Vec::new();

    for slot in SliceSlot::ALL {
        let slot_index = slot.index();
        if let Some(lut) = &program.slots[slot_index].lut
            && let Some(function_name) = encode_lut_function_name(lut, site_def, slot)
        {
            requests.push(RequestedConfig {
                cfg_name: slot.lut_config_name().to_string(),
                function_name,
            });
            requests.push(RequestedConfig {
                cfg_name: slot.lut_mux_config_name().to_string(),
                function_name: slot.lut_config_name().to_string(),
            });
        }
        if let Some(ff) = &program.slots[slot_index].ff {
            requests.push(RequestedConfig {
                cfg_name: slot.ff_config_name().to_string(),
                function_name: "#FF".to_string(),
            });
            requests.push(RequestedConfig {
                cfg_name: slot.init_config_name().to_string(),
                function_name: ff.init.as_config_value().to_string(),
            });
            requests.extend(slice_ff_data_requests(ff, slot));
        }
    }

    if let Some(lut) = &program.slots[0].lut
        && matches!(lut.output_usage, SliceLutOutputUsage::RoutedOutput)
    {
        requests.push(RequestedConfig {
            cfg_name: SliceSlot::X.lut_used_config_name().to_string(),
            function_name: "0".to_string(),
        });
    }
    if let Some(lut) = &program.slots[1].lut
        && matches!(lut.output_usage, SliceLutOutputUsage::RoutedOutput)
    {
        requests.push(RequestedConfig {
            cfg_name: SliceSlot::Y.lut_used_config_name().to_string(),
            function_name: "0".to_string(),
        });
    }

    if program.has_sequential() {
        requests.push(RequestedConfig {
            cfg_name: SliceSequentialConfigKey::ClockInvert.as_str().to_string(),
            function_name: "1".to_string(),
        });
        if program.clock_enable_mode == SliceClockEnableMode::DirectCe {
            requests.push(RequestedConfig {
                cfg_name: SliceSequentialConfigKey::ClockEnableMux
                    .as_str()
                    .to_string(),
                function_name: "CE".to_string(),
            });
        }
        requests.push(RequestedConfig {
            cfg_name: SliceSequentialConfigKey::SyncAttr.as_str().to_string(),
            function_name: "ASYNC".to_string(),
        });
        if program.set_reset_mode == SliceSetResetMode::ActiveLowShared {
            requests.push(RequestedConfig {
                cfg_name: SliceSequentialConfigKey::SetResetMux.as_str().to_string(),
                function_name: "SR_B".to_string(),
            });
            requests.push(RequestedConfig {
                cfg_name: SliceSequentialConfigKey::SetResetFfMux.as_str().to_string(),
                function_name: "0".to_string(),
            });
        }
    }

    dedup_requests(requests)
}

fn emit_block_ram_requests(program: &BlockRamProgram, site_def: &SiteDef) -> Vec<RequestedConfig> {
    let mut requests = Vec::new();

    if let Some(port_a_attr) = &program.port_a_attr {
        requests.push(RequestedConfig {
            cfg_name: "PORTA_ATTR".to_string(),
            function_name: port_a_attr.clone(),
        });
    }
    if let Some(port_b_attr) = &program.port_b_attr {
        requests.push(RequestedConfig {
            cfg_name: "PORTB_ATTR".to_string(),
            function_name: port_b_attr.clone(),
        });
    }

    if program.wea_used {
        requests.push(RequestedConfig {
            cfg_name: "WEAMUX".to_string(),
            function_name: "WEA".to_string(),
        });
    }
    if program.web_used {
        requests.push(RequestedConfig {
            cfg_name: "WEBMUX".to_string(),
            function_name: "WEB".to_string(),
        });
    }
    if program.ena_used {
        requests.push(RequestedConfig {
            cfg_name: "ENAMUX".to_string(),
            function_name: "ENA".to_string(),
        });
    }
    if program.enb_used {
        requests.push(RequestedConfig {
            cfg_name: "ENBMUX".to_string(),
            function_name: "ENB".to_string(),
        });
    }
    if program.rsta_used {
        requests.push(RequestedConfig {
            cfg_name: "RSTAMUX".to_string(),
            function_name: "RSTA".to_string(),
        });
    }
    if program.rstb_used {
        requests.push(RequestedConfig {
            cfg_name: "RSTBMUX".to_string(),
            function_name: "RSTB".to_string(),
        });
    }
    if program.clka_used {
        requests.push(RequestedConfig {
            cfg_name: "CLKAMUX".to_string(),
            function_name: "CLK".to_string(),
        });
    }
    if program.clkb_used {
        requests.push(RequestedConfig {
            cfg_name: "CLKBMUX".to_string(),
            function_name: "CLK".to_string(),
        });
    }

    for (cfg_name, function_name) in &program.init_words {
        if site_def.config_element(cfg_name).is_some() {
            requests.push(RequestedConfig {
                cfg_name: cfg_name.clone(),
                function_name: function_name.clone(),
            });
        }
    }

    dedup_requests(requests)
}

fn slice_ff_data_requests(ff: &SequentialProgram, slot: SliceSlot) -> Vec<RequestedConfig> {
    match ff.data_path {
        SliceFfDataPath::LocalLut => vec![RequestedConfig {
            cfg_name: slot.data_mux_config_name().to_string(),
            function_name: "1".to_string(),
        }],
        SliceFfDataPath::SiteBypass => vec![
            RequestedConfig {
                cfg_name: slot.data_mux_config_name().to_string(),
                function_name: "0".to_string(),
            },
            RequestedConfig {
                cfg_name: slot.bypass_mux_config_name().to_string(),
                function_name: slot.bypass_function_name().to_string(),
            },
        ],
    }
}

fn encode_lut_function_name(
    lut: &LutProgram,
    site_def: &SiteDef,
    slot: SliceSlot,
) -> Option<String> {
    let site_table_bits = site_truth_table_bits(site_def, slot.lut_config_name())?;
    let expanded_bits = expand_truth_table_bits(&lut.truth_table_bits, site_table_bits);
    Some(format_truth_table_literal(&expanded_bits))
}

fn emit_iob_requests(program: &IobProgram) -> Vec<RequestedConfig> {
    let mut requests = vec![RequestedConfig {
        cfg_name: "IOATTRBOX".to_string(),
        function_name: "LVTTL".to_string(),
    }];
    if program.input_used {
        requests.push(RequestedConfig {
            cfg_name: "IMUX".to_string(),
            function_name: "1".to_string(),
        });
    }
    if program.output_used {
        requests.push(RequestedConfig {
            cfg_name: "OMUX".to_string(),
            function_name: "O".to_string(),
        });
        requests.push(RequestedConfig {
            cfg_name: "OUTMUX".to_string(),
            function_name: "1".to_string(),
        });
        requests.push(RequestedConfig {
            cfg_name: "DRIVEATTRBOX".to_string(),
            function_name: "12".to_string(),
        });
        requests.push(RequestedConfig {
            cfg_name: "SLEW".to_string(),
            function_name: "SLOW".to_string(),
        });
    }
    dedup_requests(requests)
}

fn expand_truth_table_bits(bits: &[u8], target_width: usize) -> Vec<u8> {
    if bits.is_empty() {
        return vec![0; target_width];
    }
    if target_width <= bits.len() {
        return bits.iter().copied().take(target_width).collect();
    }

    (0..target_width)
        .map(|index| bits[index % bits.len()])
        .collect()
}

fn site_truth_table_bits(site_def: &SiteDef, cfg_name: &str) -> Option<usize> {
    site_def
        .config_element(cfg_name)?
        .functions
        .iter()
        .filter_map(address_count)
        .max()
}

fn format_truth_table_literal(bits: &[u8]) -> String {
    let digit_count = bits.len().max(1).div_ceil(4);
    let mut digits = String::with_capacity(digit_count);
    for digit_index in (0..digit_count).rev() {
        let nibble = (0..4).fold(0u8, |value, bit_index| {
            let bit = bits.get(digit_index * 4 + bit_index).copied().unwrap_or(0) & 1;
            value | (bit << bit_index)
        });
        digits.push(match nibble {
            0..=9 => char::from(b'0' + nibble),
            10..=15 => char::from(b'A' + (nibble - 10)),
            _ => return "0x0".to_string(),
        });
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        "0x0".to_string()
    } else {
        format!("0x{digits}")
    }
}

fn dedup_requests(requests: Vec<RequestedConfig>) -> Vec<RequestedConfig> {
    let mut deduped = BTreeMap::new();
    for (index, request) in requests.into_iter().enumerate() {
        deduped.insert(request.cfg_name.clone(), (index, request));
    }
    let mut ordered = deduped.into_values().collect::<Vec<_>>();
    ordered.sort_by_key(|(index, _)| *index);
    ordered.into_iter().map(|(_, request)| request).collect()
}
