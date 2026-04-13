#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockRamKind {
    SinglePort,
    DualPort,
}

impl BlockRamKind {
    pub(crate) fn from_type_name(type_name: &str) -> Option<Self> {
        match type_name.trim() {
            name if name.eq_ignore_ascii_case("BLOCKRAM_1") => Some(Self::SinglePort),
            name if name.eq_ignore_ascii_case("BLOCKRAM_2") => Some(Self::DualPort),
            _ => None,
        }
    }

    pub(crate) fn canonical_type_name(self) -> &'static str {
        match self {
            Self::SinglePort => "BLOCKRAM_1",
            Self::DualPort => "BLOCKRAM_2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockRamPortSide {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockRamControlSignal {
    Clock,
    WriteEnable,
    Reset,
    Enable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockRamPin {
    Control {
        side: BlockRamPortSide,
        signal: BlockRamControlSignal,
    },
    DataIn {
        side: BlockRamPortSide,
        index: usize,
    },
    DataOut {
        side: BlockRamPortSide,
        index: usize,
    },
    Addr {
        side: BlockRamPortSide,
        index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockRamRouteTarget {
    pub(crate) wire_name: String,
    pub(crate) row_offset: isize,
}

impl BlockRamPin {
    pub(crate) fn parse(pin: &str) -> Option<Self> {
        let normalized = pin.trim();
        let normalized = normalized
            .strip_prefix("BRAM_")
            .or_else(|| normalized.strip_prefix("bram_"))
            .unwrap_or(normalized);

        if matches_any(normalized, &["CLK", "CKA", "CLKA"]) {
            return Some(Self::control(
                BlockRamPortSide::A,
                BlockRamControlSignal::Clock,
            ));
        }
        if matches_any(normalized, &["WE", "AWE", "WEA"]) {
            return Some(Self::control(
                BlockRamPortSide::A,
                BlockRamControlSignal::WriteEnable,
            ));
        }
        if matches_any(normalized, &["RST", "RSTA"]) {
            return Some(Self::control(
                BlockRamPortSide::A,
                BlockRamControlSignal::Reset,
            ));
        }
        if matches_any(normalized, &["EN", "ENA", "AEN", "SELA"]) {
            return Some(Self::control(
                BlockRamPortSide::A,
                BlockRamControlSignal::Enable,
            ));
        }
        if matches_any(normalized, &["CLKB", "CKB"]) {
            return Some(Self::control(
                BlockRamPortSide::B,
                BlockRamControlSignal::Clock,
            ));
        }
        if matches_any(normalized, &["WEB", "BWE"]) {
            return Some(Self::control(
                BlockRamPortSide::B,
                BlockRamControlSignal::WriteEnable,
            ));
        }
        if matches_any(normalized, &["RSTB"]) {
            return Some(Self::control(
                BlockRamPortSide::B,
                BlockRamControlSignal::Reset,
            ));
        }
        if matches_any(normalized, &["ENB", "BEN", "SELB"]) {
            return Some(Self::control(
                BlockRamPortSide::B,
                BlockRamControlSignal::Enable,
            ));
        }
        if matches_any(normalized, &["DI", "DIA"]) {
            return Some(Self::DataIn {
                side: BlockRamPortSide::A,
                index: 0,
            });
        }
        if matches_any(normalized, &["DO", "DOA"]) {
            return Some(Self::DataOut {
                side: BlockRamPortSide::A,
                index: 0,
            });
        }
        if matches_any(normalized, &["DIB"]) {
            return Some(Self::DataIn {
                side: BlockRamPortSide::B,
                index: 0,
            });
        }
        if matches_any(normalized, &["DOB"]) {
            return Some(Self::DataOut {
                side: BlockRamPortSide::B,
                index: 0,
            });
        }

        if let Some(index) = parse_indexed_pin(normalized, "DI") {
            return Some(Self::DataIn {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "DO") {
            return Some(Self::DataOut {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "ADDR") {
            return Some(Self::Addr {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "DIA") {
            return Some(Self::DataIn {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "DOA") {
            return Some(Self::DataOut {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "ADDRA") {
            return Some(Self::Addr {
                side: BlockRamPortSide::A,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "DIB") {
            return Some(Self::DataIn {
                side: BlockRamPortSide::B,
                index,
            });
        }
        if let Some(index) = parse_indexed_pin(normalized, "DOB") {
            return Some(Self::DataOut {
                side: BlockRamPortSide::B,
                index,
            });
        }
        parse_indexed_pin(normalized, "ADDRB").map(|index| Self::Addr {
            side: BlockRamPortSide::B,
            index,
        })
    }

    pub(crate) fn route_target(self) -> Option<BlockRamRouteTarget> {
        Some(BlockRamRouteTarget {
            wire_name: format!("BRAM_{}", self.route_pin_name()?),
            row_offset: self.row_offset()?,
        })
    }

    pub(crate) fn canonical_map_name(
        self,
        kind: BlockRamKind,
        addr_shift_a: usize,
        addr_shift_b: usize,
    ) -> Option<String> {
        match (kind, self) {
            (
                BlockRamKind::SinglePort,
                Self::Control {
                    side: BlockRamPortSide::A,
                    signal,
                },
            ) => Some(
                match signal {
                    BlockRamControlSignal::Clock => "CLK",
                    BlockRamControlSignal::WriteEnable => "WE",
                    BlockRamControlSignal::Reset => "RST",
                    BlockRamControlSignal::Enable => "EN",
                }
                .to_string(),
            ),
            (
                BlockRamKind::SinglePort,
                Self::DataIn {
                    side: BlockRamPortSide::A,
                    index,
                },
            ) => Some(format!("DI{index}")),
            (
                BlockRamKind::SinglePort,
                Self::DataOut {
                    side: BlockRamPortSide::A,
                    index,
                },
            ) => Some(format!("DO{index}")),
            (
                BlockRamKind::SinglePort,
                Self::Addr {
                    side: BlockRamPortSide::A,
                    index,
                },
            ) => Some(format!("ADDR{}", index + addr_shift_a)),
            (BlockRamKind::SinglePort, _) => None,
            (BlockRamKind::DualPort, Self::Control { side, signal }) => {
                Some(dual_port_control_name(side, signal).to_string())
            }
            (BlockRamKind::DualPort, Self::DataIn { side, index }) => Some(match side {
                BlockRamPortSide::A => format!("DIA{index}"),
                BlockRamPortSide::B => format!("DIB{index}"),
            }),
            (BlockRamKind::DualPort, Self::DataOut { side, index }) => Some(match side {
                BlockRamPortSide::A => format!("DOA{index}"),
                BlockRamPortSide::B => format!("DOB{index}"),
            }),
            (BlockRamKind::DualPort, Self::Addr { side, index }) => Some(match side {
                BlockRamPortSide::A => format!("ADDRA{}", index + addr_shift_a),
                BlockRamPortSide::B => format!("ADDRB{}", index + addr_shift_b),
            }),
        }
    }

    fn control(side: BlockRamPortSide, signal: BlockRamControlSignal) -> Self {
        Self::Control { side, signal }
    }

    fn route_pin_name(self) -> Option<String> {
        Some(match self {
            Self::Control { side, signal } => match (side, signal) {
                (BlockRamPortSide::A, BlockRamControlSignal::Clock) => "CLKA".to_string(),
                (BlockRamPortSide::A, BlockRamControlSignal::WriteEnable) => "WEA".to_string(),
                (BlockRamPortSide::A, BlockRamControlSignal::Reset) => "RSTA".to_string(),
                (BlockRamPortSide::A, BlockRamControlSignal::Enable) => "SELA".to_string(),
                (BlockRamPortSide::B, BlockRamControlSignal::Clock) => "CLKB".to_string(),
                (BlockRamPortSide::B, BlockRamControlSignal::WriteEnable) => "WEB".to_string(),
                (BlockRamPortSide::B, BlockRamControlSignal::Reset) => "RSTB".to_string(),
                (BlockRamPortSide::B, BlockRamControlSignal::Enable) => "SELB".to_string(),
            },
            Self::DataIn { side, index } => match side {
                BlockRamPortSide::A => format!("DIA{index}"),
                BlockRamPortSide::B => format!("DIB{index}"),
            },
            Self::DataOut { side, index } => match side {
                BlockRamPortSide::A => format!("DOA{index}"),
                BlockRamPortSide::B => format!("DOB{index}"),
            },
            Self::Addr { side, index } => match side {
                BlockRamPortSide::A => format!("ADDRA{}", routed_addr_index(index)?),
                BlockRamPortSide::B => format!("ADDRB{}", routed_addr_index(index)?),
            },
        })
    }

    fn row_offset(self) -> Option<isize> {
        match self {
            Self::Control {
                side: BlockRamPortSide::A,
                ..
            } => Some(-2),
            Self::Control {
                side: BlockRamPortSide::B,
                ..
            } => Some(-1),
            Self::Addr { index, .. } => Some((routed_addr_index(index)? / 4) as isize - 2),
            Self::DataOut { index, .. } => Some(index as isize % 4 - 3),
            Self::DataIn { index, .. } => match index {
                0 | 2 | 8 | 10 => Some(-3),
                1 | 3 | 9 | 11 => Some(-2),
                4 | 5 | 12 | 13 => Some(-1),
                6 | 7 | 14 | 15 => Some(0),
                _ => None,
            },
        }
    }
}

pub(crate) fn route_target(pin: &str) -> Option<BlockRamRouteTarget> {
    BlockRamPin::parse(pin)?.route_target()
}

pub(crate) fn parse_ramb4_single_port_width(type_name: &str) -> Option<usize> {
    let suffix = type_name.trim().strip_prefix("RAMB4_")?;
    if suffix.contains('_') {
        return None;
    }
    parse_ramb4_width_token(suffix)
}

pub(crate) fn parse_ramb4_dual_port_widths(type_name: &str) -> Option<(usize, usize)> {
    let suffix = type_name.trim().strip_prefix("RAMB4_")?;
    let (port_a, port_b) = suffix.split_once('_')?;
    Some((
        parse_ramb4_width_token(port_a)?,
        parse_ramb4_width_token(port_b)?,
    ))
}

pub(crate) fn block_ram_port_attr(width: usize) -> String {
    format!("{}X{width}", 4096 / width.max(1))
}

pub(crate) fn normalized_init_property_key(key: &str) -> Option<String> {
    key.get(..4)
        .filter(|prefix| prefix.eq_ignore_ascii_case("INIT"))
        .map(|_| key.to_ascii_uppercase())
}

fn dual_port_control_name(side: BlockRamPortSide, signal: BlockRamControlSignal) -> &'static str {
    match (side, signal) {
        (BlockRamPortSide::A, BlockRamControlSignal::Clock) => "CLKA",
        (BlockRamPortSide::A, BlockRamControlSignal::WriteEnable) => "WEA",
        (BlockRamPortSide::A, BlockRamControlSignal::Reset) => "RSTA",
        (BlockRamPortSide::A, BlockRamControlSignal::Enable) => "ENA",
        (BlockRamPortSide::B, BlockRamControlSignal::Clock) => "CLKB",
        (BlockRamPortSide::B, BlockRamControlSignal::WriteEnable) => "WEB",
        (BlockRamPortSide::B, BlockRamControlSignal::Reset) => "RSTB",
        (BlockRamPortSide::B, BlockRamControlSignal::Enable) => "ENB",
    }
}

fn routed_addr_index(index: usize) -> Option<usize> {
    // The sibling C++ flow routes logical ADDR[i] pins onto BRAM_ADDRA/B[11-i]
    // site wires. C++ route XML shows, for example, ADDRA_1 -> BRAM_ADDRA10
    // and ADDRA_11 -> BRAM_ADDRA0 on the same BRAM instance.
    (index < 12).then_some(11 - index)
}

fn matches_any(pin: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| pin.eq_ignore_ascii_case(alias))
}

fn parse_ramb4_width_token(token: &str) -> Option<usize> {
    let width = token.strip_prefix('S')?.parse::<usize>().ok()?;
    matches!(width, 1 | 2 | 4 | 8 | 16).then_some(width)
}

fn parse_indexed_pin(pin: &str, prefix: &str) -> Option<usize> {
    let pin = pin.trim();
    if pin.len() < prefix.len() || !pin[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    let suffix = &pin[prefix.len()..];
    if suffix.starts_with('[') && suffix.ends_with(']') {
        return suffix[1..suffix.len() - 1].parse().ok();
    }
    (!suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{
        BlockRamKind, BlockRamPin, block_ram_port_attr, normalized_init_property_key,
        parse_ramb4_dual_port_widths, parse_ramb4_single_port_width, route_target,
    };

    #[test]
    fn maps_single_port_block_ram_pins_to_cpp_style_route_targets() {
        let di0 = route_target("DI0").expect("DI0 target");
        let di = route_target("DI").expect("DI target");
        let do0 = route_target("DO").expect("DO target");
        let do14 = route_target("DO14").expect("DO14 target");
        let addr1 = route_target("ADDR1").expect("ADDR1 target");
        let addr5 = route_target("ADDR5").expect("ADDR5 target");
        let addr11 = route_target("ADDR11").expect("ADDR11 target");
        let en = route_target("EN").expect("EN target");

        assert_eq!(di.wire_name, "BRAM_DIA0");
        assert_eq!(di.row_offset, -3);
        assert_eq!(do0.wire_name, "BRAM_DOA0");
        assert_eq!(do0.row_offset, -3);
        assert_eq!(di0.wire_name, "BRAM_DIA0");
        assert_eq!(di0.row_offset, -3);
        assert_eq!(do14.wire_name, "BRAM_DOA14");
        assert_eq!(do14.row_offset, -1);
        assert_eq!(addr1.wire_name, "BRAM_ADDRA10");
        assert_eq!(addr1.row_offset, 0);
        assert_eq!(addr5.wire_name, "BRAM_ADDRA6");
        assert_eq!(addr5.row_offset, -1);
        assert_eq!(addr11.wire_name, "BRAM_ADDRA0");
        assert_eq!(addr11.row_offset, -2);
        assert_eq!(en.wire_name, "BRAM_SELA");
        assert_eq!(en.row_offset, -2);
    }

    #[test]
    fn maps_dual_port_block_ram_pins_to_segment_specific_route_targets() {
        let dia = route_target("DIA").expect("DIA target");
        let doa = route_target("DOA").expect("DOA target");
        let dib = route_target("DIB").expect("DIB target");
        let dob = route_target("DOB").expect("DOB target");
        let dia15 = route_target("DIA15").expect("DIA15 target");
        let dob5 = route_target("DOB5").expect("DOB5 target");
        let addrb0 = route_target("ADDRB0").expect("ADDRB0 target");
        let enb = route_target("ENB").expect("ENB target");

        assert_eq!(dia.wire_name, "BRAM_DIA0");
        assert_eq!(dia.row_offset, -3);
        assert_eq!(doa.wire_name, "BRAM_DOA0");
        assert_eq!(doa.row_offset, -3);
        assert_eq!(dib.wire_name, "BRAM_DIB0");
        assert_eq!(dib.row_offset, -3);
        assert_eq!(dob.wire_name, "BRAM_DOB0");
        assert_eq!(dob.row_offset, -3);
        assert_eq!(dia15.wire_name, "BRAM_DIA15");
        assert_eq!(dia15.row_offset, 0);
        assert_eq!(dob5.wire_name, "BRAM_DOB5");
        assert_eq!(dob5.row_offset, -2);
        assert_eq!(addrb0.wire_name, "BRAM_ADDRB11");
        assert_eq!(addrb0.row_offset, 0);
        assert_eq!(enb.wire_name, "BRAM_SELB");
        assert_eq!(enb.row_offset, -1);
    }

    #[test]
    fn canonicalizes_map_pin_names_via_typed_bram_pins() {
        assert_eq!(
            BlockRamPin::parse("DIA[0]").and_then(|pin| pin.canonical_map_name(
                BlockRamKind::DualPort,
                0,
                4
            )),
            Some("DIA0".to_string())
        );
        assert_eq!(
            BlockRamPin::parse("ADDRB[7]").and_then(|pin| pin.canonical_map_name(
                BlockRamKind::DualPort,
                0,
                4
            )),
            Some("ADDRB11".to_string())
        );
        assert_eq!(
            BlockRamPin::parse("CLKA").and_then(|pin| pin.canonical_map_name(
                BlockRamKind::SinglePort,
                0,
                0
            )),
            Some("CLK".to_string())
        );
    }

    #[test]
    fn parses_ramb4_type_widths_and_attrs() {
        assert_eq!(parse_ramb4_single_port_width("RAMB4_S8"), Some(8));
        assert_eq!(parse_ramb4_dual_port_widths("RAMB4_S1_S16"), Some((1, 16)));
        assert_eq!(block_ram_port_attr(16), "256X16");
    }

    #[test]
    fn normalizes_init_property_keys_case_insensitively() {
        assert_eq!(
            normalized_init_property_key("init_0f"),
            Some("INIT_0F".to_string())
        );
        assert_eq!(
            normalized_init_property_key("INIT_00"),
            Some("INIT_00".to_string())
        );
        assert_eq!(normalized_init_property_key("PORT_ATTR"), None);
    }
}
