use crate::{cil::SiteFunction, domain::PrimitiveKind};

pub(crate) fn address_count(function: &SiteFunction) -> Option<usize> {
    function
        .srams
        .iter()
        .filter_map(|sram| sram.address.map(|address| address as usize))
        .max()
        .map(|max| max + 1)
}

pub(crate) fn evaluate_equation(raw: &str, width: usize) -> Option<Vec<u8>> {
    let value = raw.trim();
    if value == "0" {
        return Some(vec![0; width]);
    }
    if value == "1" {
        return Some(vec![1; width]);
    }
    parse_bit_literal(value, width)
}

pub(crate) fn parse_bit_literal(raw: &str, width: usize) -> Option<Vec<u8>> {
    let raw = raw.trim().replace('_', "");
    if raw.is_empty() {
        return None;
    }
    if let Some((_, value)) = raw.split_once('\'') {
        return parse_verilog_literal(value, width);
    }

    let parsed = if let Some(value) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u128::from_str_radix(value, 16).ok()?
    } else if let Some(value) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
        u128::from_str_radix(value, 2).ok()?
    } else {
        raw.parse::<u128>().ok()?
    };
    Some(
        (0..width)
            .map(|index| ((parsed >> index) & 1) as u8)
            .collect(),
    )
}

pub(crate) fn parse_verilog_literal(raw: &str, width: usize) -> Option<Vec<u8>> {
    let mut chars = raw.chars();
    let radix = chars.next()?.to_ascii_lowercase();
    let digits = chars.as_str();
    let parsed = match radix {
        'h' => u128::from_str_radix(digits, 16).ok()?,
        'b' => u128::from_str_radix(digits, 2).ok()?,
        'd' => digits.parse::<u128>().ok()?,
        _ => return None,
    };
    Some(
        (0..width)
            .map(|index| ((parsed >> index) & 1) as u8)
            .collect(),
    )
}

pub(crate) fn normalized_site_lut_truth_table_bits(
    raw_init: Option<&str>,
    canonical_init: Option<&str>,
    primitive: PrimitiveKind,
    site_input_count: usize,
) -> Option<Vec<u8>> {
    let site_truth_table_bits = 1usize.checked_shl(site_input_count as u32)?;
    if let Some(raw_bits) =
        raw_init.and_then(|value| parse_compact_hex_digit_literal(value, site_truth_table_bits))
    {
        return Some(raw_bits);
    }

    let logical_truth_table_bits = logical_truth_table_bits(primitive)?;
    let logical_bits = parse_bit_literal(canonical_init?, logical_truth_table_bits)?;
    widen_truth_table_bits(&logical_bits, site_truth_table_bits)
}

pub(crate) fn parse_compact_hex_digit_literal(raw: &str, width: usize) -> Option<Vec<u8>> {
    let raw = raw.trim().replace('_', "");
    if raw.is_empty() {
        return None;
    }
    if raw.contains('\'')
        || raw.starts_with("0x")
        || raw.starts_with("0X")
        || raw.starts_with("0b")
        || raw.starts_with("0B")
    {
        return parse_bit_literal(&raw, width);
    }
    if !raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    let hex_digits = width.div_ceil(4);
    let expanded = raw.chars().cycle().take(hex_digits).collect::<String>();
    parse_bit_literal(&format!("0x{expanded}"), width)
}

fn widen_truth_table_bits(bits: &[u8], width: usize) -> Option<Vec<u8>> {
    if bits.is_empty() || width == 0 || !width.is_multiple_of(bits.len()) {
        return None;
    }
    Some(bits.iter().copied().cycle().take(width).collect())
}

fn logical_truth_table_bits(primitive: PrimitiveKind) -> Option<usize> {
    let inputs = match primitive {
        PrimitiveKind::Lut {
            inputs: Some(inputs),
        } => inputs,
        PrimitiveKind::Lut { inputs: None }
        | PrimitiveKind::FlipFlop
        | PrimitiveKind::Latch
        | PrimitiveKind::Constant(_)
        | PrimitiveKind::Buffer
        | PrimitiveKind::Io
        | PrimitiveKind::GlobalClockBuffer
        | PrimitiveKind::Generic
        | PrimitiveKind::Unknown => return None,
    };
    1usize.checked_shl(inputs as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        normalized_site_lut_truth_table_bits, parse_bit_literal, parse_compact_hex_digit_literal,
        parse_verilog_literal,
    };
    use crate::domain::PrimitiveKind;

    #[test]
    fn parses_hex_literals_in_lsb_first_order() {
        assert_eq!(parse_bit_literal("0xA", 4), Some(vec![0, 1, 0, 1]));
        assert_eq!(parse_bit_literal("0x8000", 16).unwrap()[0], 0);
        assert_eq!(parse_bit_literal("0x8000", 16).unwrap()[15], 1);
        assert_eq!(parse_bit_literal("0x0001", 16).unwrap()[0], 1);
    }

    #[test]
    fn parses_verilog_literals_in_lsb_first_order() {
        assert_eq!(parse_verilog_literal("hA", 4), Some(vec![0, 1, 0, 1]));
        assert_eq!(parse_verilog_literal("b1010", 4), Some(vec![0, 1, 0, 1]));
    }

    #[test]
    fn expands_compact_hex_digit_literals_to_full_site_truth_table() {
        assert_eq!(
            parse_compact_hex_digit_literal("10", 16).map(|bits| bits_to_hex(&bits)),
            Some("0x1010".to_string())
        );
        assert_eq!(
            parse_compact_hex_digit_literal("12", 16).map(|bits| bits_to_hex(&bits)),
            Some("0x1212".to_string())
        );
        assert_eq!(
            parse_compact_hex_digit_literal("15", 16).map(|bits| bits_to_hex(&bits)),
            Some("0x1515".to_string())
        );
    }

    #[test]
    fn prefers_raw_init_when_normalizing_site_lut_truth_table() {
        let bits = normalized_site_lut_truth_table_bits(
            Some("12"),
            Some("0xC"),
            PrimitiveKind::Lut { inputs: Some(2) },
            4,
        )
        .expect("normalized bits");
        assert_eq!(bits_to_hex(&bits), "0x1212");
    }

    #[test]
    fn widens_canonical_lut_init_to_full_site_truth_table() {
        let bits = normalized_site_lut_truth_table_bits(
            None,
            Some("0xC"),
            PrimitiveKind::Lut { inputs: Some(2) },
            4,
        )
        .expect("normalized bits");
        assert_eq!(bits_to_hex(&bits), "0xCCCC");
    }

    fn bits_to_hex(bits: &[u8]) -> String {
        let digits = bits.len().div_ceil(4);
        let mut text = String::with_capacity(digits + 2);
        text.push_str("0x");
        for digit_index in (0..digits).rev() {
            let mut nibble = 0u8;
            for bit_index in 0..4 {
                let bit = bits.get(digit_index * 4 + bit_index).copied().unwrap_or(0) & 1;
                nibble |= bit << bit_index;
            }
            text.push(match nibble {
                0..=9 => char::from(b'0' + nibble),
                10..=15 => char::from(b'A' + (nibble - 10)),
                _ => '0',
            });
        }
        text
    }
}
