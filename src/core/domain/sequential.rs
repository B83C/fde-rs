use super::ascii::trimmed_eq_ignore_ascii_case;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequentialInitValue {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequentialCellType {
    DffHq,
    DffRhq,
    DffShq,
    DffNhq,
    DffNrhq,
    DffNshq,
    EdffHq,
    EdffTrhq,
    EdffTshq,
    TlatHq,
}

impl SequentialCellType {
    pub fn from_type_name(type_name: &str) -> Option<Self> {
        let type_name = type_name.trim();
        if type_name.is_empty() {
            return None;
        }

        [
            ("DFFHQ", Self::DffHq),
            ("DFFRHQ", Self::DffRhq),
            ("DFFSHQ", Self::DffShq),
            ("DFFNHQ", Self::DffNhq),
            ("DFFNRHQ", Self::DffNrhq),
            ("DFFNSHQ", Self::DffNshq),
            ("EDFFHQ", Self::EdffHq),
            ("EDFFTRHQ", Self::EdffTrhq),
            ("EDFFTSHQ", Self::EdffTshq),
            ("TLATHQ", Self::TlatHq),
        ]
        .into_iter()
        .find_map(|(name, kind)| trimmed_eq_ignore_ascii_case(type_name, name).then_some(kind))
    }

    pub const fn default_init_value(self) -> SequentialInitValue {
        match self {
            Self::DffShq | Self::DffNshq | Self::EdffTshq => SequentialInitValue::High,
            Self::DffHq
            | Self::DffRhq
            | Self::DffNhq
            | Self::DffNrhq
            | Self::EdffHq
            | Self::EdffTrhq
            | Self::TlatHq => SequentialInitValue::Low,
        }
    }

    pub const fn clock_is_inverted_by_default(self) -> bool {
        matches!(self, Self::DffNhq | Self::DffNrhq | Self::DffNshq)
    }
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

    pub fn from_explicit_or_type_name(
        explicit_init: Option<&str>,
        type_name: &str,
    ) -> Option<Self> {
        explicit_init
            .and_then(Self::parse)
            .or_else(|| Self::infer_from_type_name(type_name))
    }

    pub fn infer_from_type_name(type_name: &str) -> Option<Self> {
        SequentialCellType::from_type_name(type_name).map(SequentialCellType::default_init_value)
    }
}

#[cfg(test)]
mod tests {
    use super::{SequentialCellType, SequentialInitValue};

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

    #[test]
    fn infers_cpp_compatible_defaults_from_sequential_type_names() {
        assert_eq!(
            SequentialInitValue::infer_from_type_name("DFFRHQ"),
            Some(SequentialInitValue::Low)
        );
        assert_eq!(
            SequentialInitValue::infer_from_type_name("DFFSHQ"),
            Some(SequentialInitValue::High)
        );
        assert_eq!(
            SequentialInitValue::infer_from_type_name("EDFFTSHQ"),
            Some(SequentialInitValue::High)
        );
        assert_eq!(
            SequentialInitValue::infer_from_type_name("TLATHQ"),
            Some(SequentialInitValue::Low)
        );
        assert_eq!(SequentialInitValue::infer_from_type_name("LUT4"), None);
    }

    #[test]
    fn classifies_known_sequential_cell_types() {
        assert_eq!(
            SequentialCellType::from_type_name("dffnshq"),
            Some(SequentialCellType::DffNshq)
        );
        assert_eq!(
            SequentialCellType::from_type_name("edfftrhq"),
            Some(SequentialCellType::EdffTrhq)
        );
        assert_eq!(SequentialCellType::from_type_name("LUT4"), None);
    }

    #[test]
    fn identifies_inverted_clock_defaults_from_cell_type() {
        assert!(SequentialCellType::DffNhq.clock_is_inverted_by_default());
        assert!(SequentialCellType::DffNrhq.clock_is_inverted_by_default());
        assert!(SequentialCellType::DffNshq.clock_is_inverted_by_default());
        assert!(!SequentialCellType::DffHq.clock_is_inverted_by_default());
    }
}
