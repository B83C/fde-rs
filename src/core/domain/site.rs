use super::ascii::trimmed_eq_ignore_ascii_case;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum SiteKind {
    #[serde(rename = "SLICE")]
    LogicSlice,
    #[serde(rename = "BRAM")]
    BlockRam,
    #[serde(rename = "IOB")]
    Iob,
    #[serde(rename = "GCLKIOB")]
    GclkIob,
    #[serde(rename = "GCLK")]
    Gclk,
    #[serde(rename = "CONST")]
    Const,
    #[serde(rename = "UNPLACED")]
    Unplaced,
    #[default]
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl SiteKind {
    pub fn classify(raw: &str) -> Self {
        if trimmed_eq_ignore_ascii_case(raw, "SLICE") {
            Self::LogicSlice
        } else if trimmed_eq_ignore_ascii_case(raw, "BRAM")
            || trimmed_eq_ignore_ascii_case(raw, "BLOCKRAM")
            || trimmed_eq_ignore_ascii_case(raw, "BRAM16")
        {
            Self::BlockRam
        } else if trimmed_eq_ignore_ascii_case(raw, "IOB") {
            Self::Iob
        } else if trimmed_eq_ignore_ascii_case(raw, "GCLKIOB") {
            Self::GclkIob
        } else if trimmed_eq_ignore_ascii_case(raw, "GCLK") {
            Self::Gclk
        } else if trimmed_eq_ignore_ascii_case(raw, "CONST") {
            Self::Const
        } else if trimmed_eq_ignore_ascii_case(raw, "UNPLACED") {
            Self::Unplaced
        } else {
            Self::Unknown
        }
    }

    pub fn is_logic_slice(self) -> bool {
        matches!(self, Self::LogicSlice)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LogicSlice => "SLICE",
            Self::BlockRam => "BRAM",
            Self::Iob => "IOB",
            Self::GclkIob => "GCLKIOB",
            Self::Gclk => "GCLK",
            Self::Const => "CONST",
            Self::Unplaced => "UNPLACED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

impl FromStr for SiteKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::classify(value))
    }
}

impl From<&str> for SiteKind {
    fn from(value: &str) -> Self {
        Self::classify(value)
    }
}

impl From<String> for SiteKind {
    fn from(value: String) -> Self {
        Self::classify(&value)
    }
}

impl fmt::Display for SiteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceSlot {
    X,
    Y,
}

impl SliceSlot {
    pub const ALL: [Self; 2] = [Self::X, Self::Y];

    pub const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
        }
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::X),
            1 => Some(Self::Y),
            _ => None,
        }
    }

    pub const fn lut_config_name(self) -> &'static str {
        match self {
            Self::X => "F",
            Self::Y => "G",
        }
    }

    pub const fn lut_mux_config_name(self) -> &'static str {
        match self {
            Self::X => "FXMUX",
            Self::Y => "GYMUX",
        }
    }

    pub const fn lut_used_config_name(self) -> &'static str {
        match self {
            Self::X => "XUSED",
            Self::Y => "YUSED",
        }
    }

    pub const fn ff_config_name(self) -> &'static str {
        match self {
            Self::X => "FFX",
            Self::Y => "FFY",
        }
    }

    pub const fn init_config_name(self) -> &'static str {
        match self {
            Self::X => "INITX",
            Self::Y => "INITY",
        }
    }

    pub const fn data_mux_config_name(self) -> &'static str {
        match self {
            Self::X => "DXMUX",
            Self::Y => "DYMUX",
        }
    }

    pub const fn bypass_mux_config_name(self) -> &'static str {
        match self {
            Self::X => "BXMUX",
            Self::Y => "BYMUX",
        }
    }

    pub const fn bypass_function_name(self) -> &'static str {
        match self {
            Self::X => "BX",
            Self::Y => "BY",
        }
    }

    pub const fn register_output_pin(self) -> &'static str {
        match self {
            Self::X => "XQ",
            Self::Y => "YQ",
        }
    }

    pub const fn lut_output_pin(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
        }
    }

    pub fn lut_input_pin(self, physical_index: usize) -> String {
        match self {
            Self::X => format!("F{}", physical_index + 1),
            Self::Y => format!("G{}", physical_index + 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceSequentialConfigKey {
    ClockInvert,
    ClockEnableMux,
    SyncAttr,
    SetResetMux,
    SetResetFfMux,
}

impl SliceSequentialConfigKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClockInvert => "CKINV",
            Self::ClockEnableMux => "CEMUX",
            Self::SyncAttr => "SYNC_ATTR",
            Self::SetResetMux => "SRMUX",
            Self::SetResetFfMux => "SRFFMUX",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SiteKind, SliceSequentialConfigKey, SliceSlot};

    #[test]
    fn classifies_known_site_kinds() {
        assert_eq!(SiteKind::classify("slice"), SiteKind::LogicSlice);
        assert_eq!(SiteKind::classify("blockram"), SiteKind::BlockRam);
        assert_eq!(SiteKind::classify("IOB"), SiteKind::Iob);
        assert_eq!(SiteKind::classify("gclkiob"), SiteKind::GclkIob);
        assert_eq!(SiteKind::classify("nope"), SiteKind::Unknown);
    }

    #[test]
    fn slice_slot_maps_to_stable_config_names() {
        assert_eq!(SliceSlot::X.ff_config_name(), "FFX");
        assert_eq!(SliceSlot::Y.init_config_name(), "INITY");
        assert_eq!(SliceSlot::X.bypass_function_name(), "BX");
        assert_eq!(SliceSlot::Y.lut_input_pin(2), "G3");
    }

    #[test]
    fn slice_shared_sequential_config_keys_are_typed() {
        assert_eq!(SliceSequentialConfigKey::ClockInvert.as_str(), "CKINV");
        assert_eq!(SliceSequentialConfigKey::SetResetFfMux.as_str(), "SRFFMUX");
    }
}
