#[cfg(test)]
use crate::domain::{is_dedicated_clock_wire_name, is_hex_like_wire_name, is_long_wire_name};
use crate::resource::{
    Arch,
    routing::{CanonicalWireFamily, WireId, WireInterner},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WireBounds {
    pub(crate) min_x: usize,
    pub(crate) max_x: usize,
    pub(crate) min_y: usize,
    pub(crate) max_y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteNodeClass {
    Clock,
    Long,
    Hex,
    Single,
    Source,
    Sink,
}

#[cfg(test)]
pub(crate) fn canonical_indexed_wire(raw: &str) -> Option<(&'static str, usize)> {
    for prefix in [
        "LEFT_LLH",
        "RIGHT_LLH",
        "TOP_LLV",
        "BOT_LLV",
        "LEFT_E",
        "RIGHT_W",
        "TOP_S",
        "BOT_N",
        "LEFT_H6E",
        "RIGHT_H6W",
        "TOP_V6S",
        "BOT_V6N",
        "LEFT_V6N",
        "RIGHT_V6N",
        "LEFT_V6S",
        "RIGHT_V6S",
        "TOP_H6E",
        "BOT_H6E",
        "TOP_H6W",
        "BOT_H6W",
        "LLH",
        "LLV",
        "H6M",
        "H6E",
        "H6W",
        "V6N",
        "V6S",
        "V6M",
        "E",
        "W",
        "N",
        "S",
    ] {
        let Some(value) = raw.strip_prefix(prefix) else {
            continue;
        };
        let Ok(index) = value.parse::<usize>() else {
            continue;
        };
        let canonical = match prefix {
            "LEFT_LLH" | "RIGHT_LLH" => "LLH",
            "TOP_LLV" | "BOT_LLV" => "LLV",
            "LEFT_E" => "E",
            "RIGHT_W" => "W",
            "TOP_S" => "S",
            "BOT_N" => "N",
            "LEFT_H6E" | "TOP_H6E" | "BOT_H6E" => "H6E",
            "RIGHT_H6W" | "TOP_H6W" | "BOT_H6W" => "H6W",
            "TOP_V6S" | "LEFT_V6S" | "RIGHT_V6S" => "V6S",
            "BOT_V6N" | "LEFT_V6N" | "RIGHT_V6N" => "V6N",
            other => other,
        };
        return Some((canonical, index));
    }
    None
}

#[cfg(test)]
pub(crate) fn wire_bounds(arch: &Arch, x: usize, y: usize, raw: &str) -> Option<WireBounds> {
    let (family, _) = canonical_indexed_wire(raw)?;
    Some(match family {
        // FDE coordinates use x as the row axis and y as the column axis.
        // Horizontal channels therefore vary y, vertical channels vary x.
        "E" => span_bounds(arch, x, y, 0, 1),
        "W" => span_bounds(arch, x, y, 0, -1),
        "N" => span_bounds(arch, x, y, -1, 0),
        "S" => span_bounds(arch, x, y, 1, 0),
        "H6E" => span_bounds(arch, x, y, 0, 6),
        "H6W" => span_bounds(arch, x, y, 0, -6),
        "H6M" => centered_span_bounds(arch, x, y, 6, true),
        "V6N" => span_bounds(arch, x, y, -6, 0),
        "V6S" => span_bounds(arch, x, y, 6, 0),
        "V6M" => centered_span_bounds(arch, x, y, 6, false),
        "LLH" => WireBounds {
            min_x: x.min(arch.width.saturating_sub(1)),
            max_x: x.min(arch.width.saturating_sub(1)),
            min_y: 0,
            max_y: arch.height.saturating_sub(1),
        },
        "LLV" => WireBounds {
            min_x: 0,
            max_x: arch.width.saturating_sub(1),
            min_y: y.min(arch.height.saturating_sub(1)),
            max_y: y.min(arch.height.saturating_sub(1)),
        },
        _ => return None,
    })
}

pub(crate) fn wire_bounds_for_wire(
    arch: &Arch,
    x: usize,
    y: usize,
    wires: &WireInterner,
    wire: WireId,
) -> Option<WireBounds> {
    let family = wires.metadata(wire).family()?;
    Some(match family {
        CanonicalWireFamily::E => span_bounds(arch, x, y, 0, 1),
        CanonicalWireFamily::W => span_bounds(arch, x, y, 0, -1),
        CanonicalWireFamily::N => span_bounds(arch, x, y, -1, 0),
        CanonicalWireFamily::S => span_bounds(arch, x, y, 1, 0),
        CanonicalWireFamily::H6E => span_bounds(arch, x, y, 0, 6),
        CanonicalWireFamily::H6W => span_bounds(arch, x, y, 0, -6),
        CanonicalWireFamily::H6M => centered_span_bounds(arch, x, y, 6, true),
        CanonicalWireFamily::V6N => span_bounds(arch, x, y, -6, 0),
        CanonicalWireFamily::V6S => span_bounds(arch, x, y, 6, 0),
        CanonicalWireFamily::V6M => centered_span_bounds(arch, x, y, 6, false),
        CanonicalWireFamily::Llh => WireBounds {
            min_x: x.min(arch.width.saturating_sub(1)),
            max_x: x.min(arch.width.saturating_sub(1)),
            min_y: 0,
            max_y: arch.height.saturating_sub(1),
        },
        CanonicalWireFamily::Llv => WireBounds {
            min_x: 0,
            max_x: arch.width.saturating_sub(1),
            min_y: y.min(arch.height.saturating_sub(1)),
            max_y: y.min(arch.height.saturating_sub(1)),
        },
    })
}

pub(crate) fn tile_distance(x0: usize, y0: usize, x1: usize, y1: usize) -> usize {
    x0.abs_diff(x1) + y0.abs_diff(y1)
}

#[cfg(test)]
pub(crate) fn route_node_class(
    raw: &str,
    bounds: Option<WireBounds>,
    has_successors: bool,
) -> RouteNodeClass {
    if is_dedicated_clock_wire_name(raw) {
        return RouteNodeClass::Clock;
    }

    let length = bounds
        .map(|bounds| bounds.max_x - bounds.min_x + bounds.max_y - bounds.min_y)
        .unwrap_or(0);
    if is_long_wire_name(raw) && length != 0 {
        return RouteNodeClass::Long;
    }
    if is_hex_like_wire_name(raw) {
        return RouteNodeClass::Hex;
    }
    if matches!(length, 1 | 2) {
        return RouteNodeClass::Single;
    }
    if has_successors {
        RouteNodeClass::Source
    } else {
        RouteNodeClass::Sink
    }
}

pub(crate) fn route_node_class_for_wire(
    wires: &WireInterner,
    wire: WireId,
    bounds: Option<WireBounds>,
    has_successors: bool,
) -> RouteNodeClass {
    let metadata = wires.metadata(wire);
    if metadata.is_dedicated_clock() {
        return RouteNodeClass::Clock;
    }

    let length = bounds
        .map(|bounds| bounds.max_x - bounds.min_x + bounds.max_y - bounds.min_y)
        .unwrap_or(0);
    if metadata.is_long() && length != 0 {
        return RouteNodeClass::Long;
    }
    if metadata.is_hex_like() {
        return RouteNodeClass::Hex;
    }
    if matches!(length, 1 | 2) {
        return RouteNodeClass::Single;
    }
    if has_successors {
        RouteNodeClass::Source
    } else {
        RouteNodeClass::Sink
    }
}

pub(crate) fn route_node_base_cost(class: RouteNodeClass) -> usize {
    match class {
        // Keep unit cost for SOURCE / HEX / LONG, doubled cost for SINGLE,
        // half-rate cost for dedicated clock sources, and zero for SINK.
        // Scale by 2 to stay in integer space.
        RouteNodeClass::Clock => 1,
        RouteNodeClass::Long | RouteNodeClass::Hex | RouteNodeClass::Source => 2,
        RouteNodeClass::Single => 4,
        RouteNodeClass::Sink => 0,
    }
}

#[cfg(test)]
pub(crate) fn is_exclusive_site_output_wire(raw: &str) -> bool {
    raw.starts_with('S')
        && raw[1..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
        && matches!(
            raw,
            value if value.ends_with("_XQ")
                || value.ends_with("_YQ")
                || value.ends_with("_X")
                || value.ends_with("_Y")
        )
}

fn span_bounds(arch: &Arch, x: usize, y: usize, dx: isize, dy: isize) -> WireBounds {
    let target_x = offset_clamped(x, dx, arch.width.saturating_sub(1));
    let target_y = offset_clamped(y, dy, arch.height.saturating_sub(1));
    WireBounds {
        min_x: x.min(target_x),
        max_x: x.max(target_x),
        min_y: y.min(target_y),
        max_y: y.max(target_y),
    }
}

fn centered_span_bounds(
    arch: &Arch,
    x: usize,
    y: usize,
    radius: usize,
    horizontal: bool,
) -> WireBounds {
    if horizontal {
        WireBounds {
            min_x: x.min(arch.width.saturating_sub(1)),
            max_x: x.min(arch.width.saturating_sub(1)),
            min_y: y.saturating_sub(radius),
            max_y: y.saturating_add(radius).min(arch.height.saturating_sub(1)),
        }
    } else {
        WireBounds {
            min_x: x.saturating_sub(radius),
            max_x: x.saturating_add(radius).min(arch.width.saturating_sub(1)),
            min_y: y.min(arch.height.saturating_sub(1)),
            max_y: y.min(arch.height.saturating_sub(1)),
        }
    }
}

fn offset_clamped(origin: usize, delta: isize, max: usize) -> usize {
    if delta.is_negative() {
        origin.saturating_sub(delta.unsigned_abs())
    } else {
        origin.saturating_add(delta as usize).min(max)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RouteNodeClass, WireBounds, canonical_indexed_wire, is_exclusive_site_output_wire,
        route_node_base_cost, route_node_class, wire_bounds,
    };
    use crate::resource::Arch;
    use std::collections::BTreeMap;

    fn mini_arch() -> Arch {
        Arch {
            width: 35,
            height: 55,
            tiles: BTreeMap::new(),
            ..Arch::default()
        }
    }

    #[test]
    fn canonicalizes_edge_and_long_wire_families() {
        assert_eq!(canonical_indexed_wire("LEFT_LLH10"), Some(("LLH", 10)));
        assert_eq!(canonical_indexed_wire("RIGHT_H6W6"), Some(("H6W", 6)));
        assert_eq!(canonical_indexed_wire("V6M3"), Some(("V6M", 3)));
        assert_eq!(canonical_indexed_wire("S17"), Some(("S", 17)));
    }

    #[test]
    fn derives_directional_bounds_from_wire_family() {
        let arch = mini_arch();
        assert_eq!(
            wire_bounds(&arch, 16, 11, "W16"),
            Some(WireBounds {
                min_x: 16,
                max_x: 16,
                min_y: 10,
                max_y: 11,
            })
        );
        assert_eq!(
            wire_bounds(&arch, 16, 11, "H6W6"),
            Some(WireBounds {
                min_x: 16,
                max_x: 16,
                min_y: 5,
                max_y: 11,
            })
        );
        assert_eq!(
            wire_bounds(&arch, 16, 11, "LLH0"),
            Some(WireBounds {
                min_x: 16,
                max_x: 16,
                min_y: 0,
                max_y: 54,
            })
        );
    }

    #[test]
    fn classifies_route_node_cost_families_for_router() {
        let arch = mini_arch();
        let single = wire_bounds(&arch, 16, 11, "W16");
        let hex = wire_bounds(&arch, 16, 11, "H6W6");
        let long = wire_bounds(&arch, 16, 11, "LLH0");

        assert_eq!(
            route_node_class("W16", single, true),
            RouteNodeClass::Single
        );
        assert_eq!(route_node_class("H6W6", hex, true), RouteNodeClass::Hex);
        assert_eq!(route_node_class("LLH0", long, true), RouteNodeClass::Long);
        assert_eq!(
            route_node_class("S0_XQ", None, true),
            RouteNodeClass::Source
        );
        assert_eq!(
            route_node_class("S0_F_B1", None, false),
            RouteNodeClass::Sink
        );
        assert_eq!(route_node_base_cost(RouteNodeClass::Single), 4);
        assert_eq!(route_node_base_cost(RouteNodeClass::Hex), 2);
        assert_eq!(route_node_base_cost(RouteNodeClass::Sink), 0);
    }

    #[test]
    fn identifies_slice_outputs_that_require_single_local_exit() {
        assert!(is_exclusive_site_output_wire("S0_XQ"));
        assert!(is_exclusive_site_output_wire("S1_Y"));
        assert!(!is_exclusive_site_output_wire("S0_CLK_B"));
        assert!(!is_exclusive_site_output_wire("OUT4"));
        assert!(!is_exclusive_site_output_wire("E_P12"));
    }
}
