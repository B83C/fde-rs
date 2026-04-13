use crate::bitgen::literal::parse_bit_literal;
use std::collections::BTreeSet;

pub(crate) const PHYSICAL_LUT_FUNCTION_PROPERTY: &str = "fde_physical_lut_function";

const LUT_EXPR_TERMS: &[&[&str]] = &[
    &["~A1*A1", "A1", "~A1"],
    &[
        "(~A1*A1)+(~A2*A2)",
        "(A2*A1)",
        "(A2*~A1)",
        "(~A2*A1)",
        "(~A2*~A1)",
    ],
    &[
        "(~A1*A1)+(~A2*A2)+(~A3*A3)",
        "((A3*A2)*A1)",
        "((A3*A2)*~A1)",
        "((A3*~A2)*A1)",
        "((A3*~A2)*~A1)",
        "((~A3*A2)*A1)",
        "((~A3*A2)*~A1)",
        "((~A3*~A2)*A1)",
        "((~A3*~A2)*~A1)",
    ],
    &[
        "(~A1*A1)+(~A2*A2)+(~A3*A3)+(~A4*A4)",
        "(((A4*A3)*A2)*A1)",
        "(((A4*A3)*A2)*~A1)",
        "(((A4*A3)*~A2)*A1)",
        "(((A4*A3)*~A2)*~A1)",
        "(((A4*~A3)*A2)*A1)",
        "(((A4*~A3)*A2)*~A1)",
        "(((A4*~A3)*~A2)*A1)",
        "(((A4*~A3)*~A2)*~A1)",
        "(((~A4*A3)*A2)*A1)",
        "(((~A4*A3)*A2)*~A1)",
        "(((~A4*A3)*~A2)*A1)",
        "(((~A4*A3)*~A2)*~A1)",
        "(((~A4*~A3)*A2)*A1)",
        "(((~A4*~A3)*A2)*~A1)",
        "(((~A4*~A3)*~A2)*A1)",
        "(((~A4*~A3)*~A2)*~A1)",
    ],
];

pub(super) fn encode_lut_expression_literal(bits: &[u8], input_count: usize) -> String {
    let term_index = input_count.saturating_sub(1);
    let Some(terms) = LUT_EXPR_TERMS.get(term_index) else {
        return "#OFF".to_string();
    };

    let mut expr = String::new();
    let mut term_number = 1usize;
    let start_mask = if term_index == 0 { 0b0010 } else { 0b1000 };
    let digit_count = (1usize << input_count.max(1)).div_ceil(4);
    for digit_index in (0..digit_count).rev() {
        let value = (0..4).fold(0u8, |nibble, bit_index| {
            let bit = bits.get(digit_index * 4 + bit_index).copied().unwrap_or(0) & 1;
            nibble | (bit << bit_index)
        });
        let mut mask = start_mask;
        while mask != 0 {
            if (value & mask) != 0 {
                expr.push('+');
                if let Some(term) = terms.get(term_number) {
                    expr.push_str(term);
                }
            }
            term_number += 1;
            mask >>= 1;
        }
    }
    if expr.is_empty() {
        format!("#LUT:D={}", terms[0])
    } else {
        format!("#LUT:D={}", &expr[1..])
    }
}

pub(super) fn decode_lut_function(value: &str) -> Option<(String, usize)> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("#OFF") || value.is_empty() {
        return None;
    }
    if let Some(expr) = value.strip_prefix("#LUT:D=") {
        return decode_lut_expression(expr).or_else(|| decode_lut_literal(expr));
    }
    decode_lut_literal(value)
}

pub(crate) fn preserved_physical_lut_function(value: &str) -> Option<String> {
    let value = value.trim();
    matches!(value, "#LUT:D=0" | "#LUT:D=1").then(|| value.to_string())
}

fn decode_lut_expression(expr: &str) -> Option<(String, usize)> {
    let expr = expr.trim();
    if expr == "0" {
        return Some(("0x0".to_string(), 4));
    }
    if expr == "1" {
        return Some(("0xFFFF".to_string(), 4));
    }
    for (term_index, terms) in LUT_EXPR_TERMS.iter().enumerate() {
        let input_count = term_index + 1;
        if expr == terms[0] {
            return Some((
                format_truth_table_literal(&vec![0; 1usize << input_count]),
                input_count,
            ));
        }
        let tokens = expr
            .split('+')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect::<BTreeSet<_>>();
        if tokens.is_empty() {
            continue;
        }
        if !tokens.iter().all(|token| terms[1..].contains(token)) {
            continue;
        }
        let mut bits = vec![0u8; 1usize << input_count];
        let bit_count = terms[1..].len();
        for (term_index, term) in terms[1..].iter().enumerate() {
            if tokens.contains(term) {
                let bit_index = bit_count - 1 - term_index;
                bits[bit_index] = 1;
            }
        }
        return Some((format_truth_table_literal(&bits), input_count));
    }
    None
}

fn decode_lut_literal(value: &str) -> Option<(String, usize)> {
    let bit_width = if let Some(digits) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        digits.len().max(1) * 4
    } else if let Some(digits) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        digits.len().max(1)
    } else if value.contains('\'') {
        let (_, suffix) = value.split_once('\'')?;
        match suffix.chars().next()?.to_ascii_lowercase() {
            'h' => suffix[1..].len().max(1) * 4,
            'b' => suffix[1..].len().max(1),
            'd' => 16,
            _ => return None,
        }
    } else {
        16
    };
    let input_count = (1..=4)
        .find(|input_count| (1usize << input_count) >= bit_width)
        .unwrap_or(4);
    let width = 1usize << input_count;
    let bits = parse_bit_literal(value, width)?;
    Some((format_truth_table_literal(&bits), input_count))
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
            _ => '0',
        });
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        "0x0".to_string()
    } else {
        format!("0x{digits}")
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_lut_function, encode_lut_expression_literal};

    #[test]
    fn encodes_and_decodes_lut_expression_literals() {
        let bits = vec![0, 1, 0, 1];
        let encoded = encode_lut_expression_literal(&bits, 2);
        assert_eq!(encoded, "#LUT:D=(A2*A1)+(~A2*A1)");
        assert_eq!(decode_lut_function(&encoded), Some(("0xA".to_string(), 2)));
    }

    #[test]
    fn decodes_literal_forms() {
        assert_eq!(decode_lut_function("0xC"), Some(("0xC".to_string(), 2)));
        assert_eq!(
            decode_lut_function("16'h8000"),
            Some(("0x8000".to_string(), 4))
        );
    }

    #[test]
    fn decodes_non_symmetric_expression_bit_order_correctly() {
        assert_eq!(
            decode_lut_function("#LUT:D=(~A2*~A1)"),
            Some(("0x1".to_string(), 2))
        );
        assert_eq!(
            decode_lut_function("#LUT:D=(A2*A1)"),
            Some(("0x8".to_string(), 2))
        );
        assert_eq!(
            decode_lut_function("#LUT:D=((A3*~A2)*~A1)+((~A3*A2)*~A1)"),
            Some(("0x14".to_string(), 3))
        );
    }

    #[test]
    fn decodes_constant_expression_literals_emitted_by_cpp_physical_xml() {
        assert_eq!(
            decode_lut_function("#LUT:D=0"),
            Some(("0x0".to_string(), 4))
        );
        assert_eq!(
            decode_lut_function("#LUT:D=1"),
            Some(("0xFFFF".to_string(), 4))
        );
    }

    #[test]
    fn preserves_declared_lut_input_count_for_leading_zero_truth_tables() {
        let bits = vec![1, 0, 0, 0, 0, 0, 0, 0];
        let encoded = encode_lut_expression_literal(&bits, 3);
        assert_eq!(encoded, "#LUT:D=((~A3*~A2)*~A1)");
        assert_eq!(decode_lut_function(&encoded), Some(("0x1".to_string(), 3)));
    }
}
