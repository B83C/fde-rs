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

#[cfg(test)]
mod tests {
    use super::SiteKind;

    #[test]
    fn classifies_known_site_kinds() {
        assert_eq!(SiteKind::classify("slice"), SiteKind::LogicSlice);
        assert_eq!(SiteKind::classify("blockram"), SiteKind::BlockRam);
        assert_eq!(SiteKind::classify("IOB"), SiteKind::Iob);
        assert_eq!(SiteKind::classify("gclkiob"), SiteKind::GclkIob);
        assert_eq!(SiteKind::classify("nope"), SiteKind::Unknown);
    }
}
