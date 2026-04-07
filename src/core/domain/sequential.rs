use super::ascii::trimmed_eq_ignore_ascii_case;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequentialInitValue {
    Low,
    High,
}

impl SequentialInitValue {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if matches!(raw, "0" | "1'0" | "1'b0")
            || trimmed_eq_ignore_ascii_case(raw, "low")
            || trimmed_eq_ignore_ascii_case(raw, "false")
        {
            return Some(Self::Low);
        }
        if matches!(raw, "1" | "1'1" | "1'b1")
            || trimmed_eq_ignore_ascii_case(raw, "high")
            || trimmed_eq_ignore_ascii_case(raw, "true")
        {
            return Some(Self::High);
        }
        None
    }

    pub const fn as_config_value(self) -> &'static str {
        match self {
            Self::Low => "LOW",
            Self::High => "HIGH",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SequentialInitValue;

    #[test]
    fn parses_common_single_bit_init_spellings() {
        assert_eq!(
            SequentialInitValue::parse("0"),
            Some(SequentialInitValue::Low)
        );
        assert_eq!(
            SequentialInitValue::parse("1"),
            Some(SequentialInitValue::High)
        );
        assert_eq!(
            SequentialInitValue::parse("1'0"),
            Some(SequentialInitValue::Low)
        );
        assert_eq!(
            SequentialInitValue::parse("1'b1"),
            Some(SequentialInitValue::High)
        );
        assert_eq!(
            SequentialInitValue::parse("LOW"),
            Some(SequentialInitValue::Low)
        );
        assert_eq!(
            SequentialInitValue::parse("high"),
            Some(SequentialInitValue::High)
        );
        assert_eq!(SequentialInitValue::parse(""), None);
        assert_eq!(SequentialInitValue::parse("2"), None);
    }
}
