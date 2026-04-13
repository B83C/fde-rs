#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CanonicalWireFamily {
    E,
    W,
    N,
    S,
    H6E,
    H6W,
    H6M,
    V6N,
    V6S,
    V6M,
    Llh,
    Llv,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WireNameMetadata {
    family: Option<CanonicalWireFamily>,
    dedicated_clock: bool,
    clock_distribution: bool,
    clock_sink: bool,
    block_ram_clock_sink: bool,
    pad_stub: bool,
    hex_like: bool,
    long: bool,
    directional_channel: bool,
}

impl WireNameMetadata {
    pub(crate) fn family(self) -> Option<CanonicalWireFamily> {
        self.family
    }

    pub(crate) fn is_dedicated_clock(self) -> bool {
        self.dedicated_clock
    }

    pub(crate) fn is_clock_distribution(self) -> bool {
        self.clock_distribution
    }

    pub(crate) fn is_clock_sink(self) -> bool {
        self.clock_sink
    }

    pub(crate) fn is_block_ram_clock_sink(self) -> bool {
        self.block_ram_clock_sink
    }

    pub(crate) fn is_pad_stub(self) -> bool {
        self.pad_stub
    }

    pub(crate) fn is_hex_like(self) -> bool {
        self.hex_like
    }

    pub(crate) fn is_long(self) -> bool {
        self.long
    }

    pub(crate) fn is_directional_channel(self) -> bool {
        self.directional_channel
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceOutputWireKind {
    LutX,
    LutY,
    RegisterX,
    RegisterY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceControlWireKind {
    Clock,
    ClockEnable,
    SetReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SliceHalf {
    X,
    Y,
}

pub(crate) fn parse_canonical_indexed_wire(raw: &str) -> Option<(CanonicalWireFamily, usize)> {
    let prefix = [
        ("LEFT_LLH", CanonicalWireFamily::Llh),
        ("RIGHT_LLH", CanonicalWireFamily::Llh),
        ("TOP_LLV", CanonicalWireFamily::Llv),
        ("BOT_LLV", CanonicalWireFamily::Llv),
        ("LEFT_E", CanonicalWireFamily::E),
        ("RIGHT_W", CanonicalWireFamily::W),
        ("TOP_S", CanonicalWireFamily::S),
        ("BOT_N", CanonicalWireFamily::N),
        ("LEFT_H6E", CanonicalWireFamily::H6E),
        ("RIGHT_H6W", CanonicalWireFamily::H6W),
        ("TOP_V6S", CanonicalWireFamily::V6S),
        ("BOT_V6N", CanonicalWireFamily::V6N),
        ("LEFT_V6N", CanonicalWireFamily::V6N),
        ("RIGHT_V6N", CanonicalWireFamily::V6N),
        ("LEFT_V6S", CanonicalWireFamily::V6S),
        ("RIGHT_V6S", CanonicalWireFamily::V6S),
        ("TOP_H6E", CanonicalWireFamily::H6E),
        ("BOT_H6E", CanonicalWireFamily::H6E),
        ("TOP_H6W", CanonicalWireFamily::H6W),
        ("BOT_H6W", CanonicalWireFamily::H6W),
        ("LLH", CanonicalWireFamily::Llh),
        ("LLV", CanonicalWireFamily::Llv),
        ("H6M", CanonicalWireFamily::H6M),
        ("H6E", CanonicalWireFamily::H6E),
        ("H6W", CanonicalWireFamily::H6W),
        ("V6N", CanonicalWireFamily::V6N),
        ("V6S", CanonicalWireFamily::V6S),
        ("V6M", CanonicalWireFamily::V6M),
        ("E", CanonicalWireFamily::E),
        ("W", CanonicalWireFamily::W),
        ("N", CanonicalWireFamily::N),
        ("S", CanonicalWireFamily::S),
    ];

    for (candidate, family) in prefix {
        let Some(suffix) = raw.strip_prefix(candidate) else {
            continue;
        };
        let Ok(index) = suffix.parse::<usize>() else {
            continue;
        };
        return Some((family, index));
    }

    None
}

pub(crate) fn canonical_wire_family(raw: &str) -> Option<CanonicalWireFamily> {
    parse_canonical_indexed_wire(raw).map(|(family, _)| family)
}

pub(crate) fn wire_name_metadata(raw: &str) -> WireNameMetadata {
    WireNameMetadata {
        family: canonical_wire_family(raw),
        dedicated_clock: is_dedicated_clock_wire_name(raw),
        clock_distribution: is_clock_distribution_wire_name(raw),
        clock_sink: is_clock_sink_wire_name(raw),
        block_ram_clock_sink: is_block_ram_clock_sink_wire_name(raw),
        pad_stub: is_pad_stub_wire_name(raw),
        hex_like: is_hex_like_wire_name(raw),
        long: is_long_wire_name(raw),
        directional_channel: is_directional_channel_wire_name(raw),
    }
}

pub fn is_dedicated_clock_wire_name(raw: &str) -> bool {
    raw.contains("GCLK")
}

pub fn is_clock_distribution_wire_name(raw: &str) -> bool {
    is_dedicated_clock_wire_name(raw) || raw.contains("CLKV") || raw.contains("CLKC")
}

pub fn is_clock_sink_wire_name(raw: &str) -> bool {
    raw.ends_with("_CLK_B") || is_block_ram_clock_sink_wire_name(raw)
}

pub(crate) fn is_block_ram_clock_sink_wire_name(raw: &str) -> bool {
    raw.eq_ignore_ascii_case("BRAM_CLKA") || raw.eq_ignore_ascii_case("BRAM_CLKB")
}

pub fn is_pad_stub_wire_name(raw: &str) -> bool {
    raw.contains("_P")
}

pub fn is_hex_like_wire_name(raw: &str) -> bool {
    raw.contains("H6") || raw.contains("V6")
}

pub fn is_long_wire_name(raw: &str) -> bool {
    raw.contains("LLH")
        || raw.contains("LLV")
        || raw.starts_with("LH")
        || raw.starts_with("LV")
        || raw.starts_with("LEFT_LLH")
        || raw.starts_with("RIGHT_LLH")
        || raw.starts_with("TOP_LLV")
        || raw.starts_with("BOT_LLV")
}

pub fn is_directional_channel_wire_name(raw: &str) -> bool {
    matches!(raw.chars().next(), Some('N' | 'S' | 'E' | 'W'))
}

pub fn slice_output_wire_kind(raw: &str) -> Option<SliceOutputWireKind> {
    match raw {
        value if value.ends_with("_XQ") => Some(SliceOutputWireKind::RegisterX),
        value if value.ends_with("_YQ") => Some(SliceOutputWireKind::RegisterY),
        value if value.ends_with("_X") => Some(SliceOutputWireKind::LutX),
        value if value.ends_with("_Y") => Some(SliceOutputWireKind::LutY),
        _ => None,
    }
}

pub fn output_wire_index(raw: &str) -> Option<usize> {
    raw.strip_prefix("OUT")?.parse::<usize>().ok()
}

pub fn sink_output_preference(raw: &str) -> Option<usize> {
    if raw.ends_with("_O1") {
        Some(1)
    } else if raw.ends_with("_O2") {
        Some(2)
    } else {
        None
    }
}

pub(crate) fn should_skip_site_local_route_arc(site_type: &str, from: &str, to: &str) -> bool {
    matches!(site_type, "GSB_LFT")
        && to.starts_with("LEFT_O")
        && from.starts_with("LEFT_H6")
        && from.contains("_BUF")
}

pub fn normalized_slice_site_name(site_name: &str) -> &str {
    if site_name
        .strip_prefix('S')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
    {
        site_name
    } else {
        "S0"
    }
}

pub fn slice_register_output_wire_name(site_name: &str, slot: usize) -> String {
    let prefix = normalized_slice_site_name(site_name);
    match slice_half(slot) {
        SliceHalf::X => format!("{prefix}_XQ"),
        SliceHalf::Y => format!("{prefix}_YQ"),
    }
}

pub fn slice_lut_output_wire_name(site_name: &str, slot: usize) -> String {
    let prefix = normalized_slice_site_name(site_name);
    match slice_half(slot) {
        SliceHalf::X => format!("{prefix}_X"),
        SliceHalf::Y => format!("{prefix}_Y"),
    }
}

pub fn slice_lut_input_wire_prefix(site_name: &str, slot: usize) -> String {
    let prefix = normalized_slice_site_name(site_name);
    match slice_half(slot) {
        SliceHalf::X => format!("{prefix}_F_B"),
        SliceHalf::Y => format!("{prefix}_G_B"),
    }
}

pub fn slice_control_wire_name(site_name: &str, kind: SliceControlWireKind) -> String {
    let prefix = normalized_slice_site_name(site_name);
    let suffix = match kind {
        SliceControlWireKind::Clock => "CLK_B",
        SliceControlWireKind::ClockEnable => "CE_B",
        SliceControlWireKind::SetReset => "SR_B",
    };
    format!("{prefix}_{suffix}")
}

pub fn slice_register_data_wire_name(site_name: &str, slot: usize) -> String {
    let prefix = normalized_slice_site_name(site_name);
    match slice_half(slot) {
        SliceHalf::X => format!("{prefix}_BX_B"),
        SliceHalf::Y => format!("{prefix}_BY_B"),
    }
}

pub fn pin_map_property_name(logical_index: usize) -> String {
    format!("pin_map_ADR{logical_index}")
}

fn slice_half(slot: usize) -> SliceHalf {
    if slot == 0 {
        SliceHalf::X
    } else {
        SliceHalf::Y
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CanonicalWireFamily, SliceControlWireKind, SliceOutputWireKind, canonical_wire_family,
        is_clock_distribution_wire_name, is_clock_sink_wire_name, is_dedicated_clock_wire_name,
        is_directional_channel_wire_name, is_hex_like_wire_name, is_long_wire_name,
        normalized_slice_site_name, output_wire_index, parse_canonical_indexed_wire,
        pin_map_property_name, should_skip_site_local_route_arc, sink_output_preference,
        slice_control_wire_name, slice_lut_input_wire_prefix, slice_lut_output_wire_name,
        slice_output_wire_kind, slice_register_data_wire_name, slice_register_output_wire_name,
        wire_name_metadata,
    };

    #[test]
    fn classifies_route_wire_name_semantics() {
        assert!(is_dedicated_clock_wire_name("CLKB_GCLK0"));
        assert!(is_clock_distribution_wire_name("CLKV_VGCLK0"));
        assert!(is_clock_sink_wire_name("S0_CLK_B"));
        assert!(is_clock_sink_wire_name("BRAM_CLKA"));
        assert!(is_clock_sink_wire_name("BRAM_CLKB"));
        assert!(is_hex_like_wire_name("H6W6"));
        assert!(is_long_wire_name("LEFT_LLH3"));
        assert!(is_directional_channel_wire_name("N8"));
        assert_eq!(
            slice_output_wire_kind("S0_XQ"),
            Some(SliceOutputWireKind::RegisterX)
        );
        assert_eq!(output_wire_index("OUT4"), Some(4));
        assert_eq!(sink_output_preference("LEFT_O2"), Some(2));
        assert_eq!(normalized_slice_site_name("S12"), "S12");
        assert_eq!(normalized_slice_site_name("SLICE0"), "S0");
        assert_eq!(slice_register_output_wire_name("S0", 0), "S0_XQ");
        assert_eq!(slice_lut_output_wire_name("S0", 1), "S0_Y");
        assert_eq!(slice_lut_input_wire_prefix("SLICE0", 1), "S0_G_B");
        assert_eq!(
            slice_control_wire_name("S1", SliceControlWireKind::ClockEnable),
            "S1_CE_B"
        );
        assert_eq!(slice_register_data_wire_name("S1", 1), "S1_BY_B");
        assert_eq!(pin_map_property_name(2), "pin_map_ADR2");
    }

    #[test]
    fn canonicalizes_indexed_wire_families() {
        assert_eq!(
            parse_canonical_indexed_wire("RIGHT_H6W6"),
            Some((CanonicalWireFamily::H6W, 6))
        );
        assert_eq!(
            canonical_wire_family("TOP_LLV4"),
            Some(CanonicalWireFamily::Llv)
        );
    }

    #[test]
    fn derives_cached_wire_name_metadata() {
        let metadata = wire_name_metadata("CLKV_GCLK_BUFR1");
        assert!(metadata.is_dedicated_clock());
        assert!(metadata.is_clock_distribution());
        assert!(!metadata.is_directional_channel());

        let metadata = wire_name_metadata("LEFT_H6E_BUF2");
        assert_eq!(metadata.family(), None);
        assert!(metadata.is_hex_like());
        assert!(!metadata.is_clock_sink());

        let metadata = wire_name_metadata("BRAM_CLKA");
        assert!(metadata.is_clock_sink());
        assert!(metadata.is_block_ram_clock_sink());
        assert!(!metadata.is_clock_distribution());
    }

    #[test]
    fn encodes_known_site_local_arc_compatibility_skip() {
        assert!(should_skip_site_local_route_arc(
            "GSB_LFT",
            "LEFT_H6A_BUF1",
            "LEFT_O1"
        ));
        assert!(!should_skip_site_local_route_arc(
            "GSB_LFT",
            "LEFT_E_BUF3",
            "LEFT_O1"
        ));
        assert!(!should_skip_site_local_route_arc(
            "GSB_RGT",
            "LEFT_H6A_BUF1",
            "LEFT_O1"
        ));
    }
}
