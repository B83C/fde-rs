use super::ascii::{trimmed_contains_ignore_ascii_case, trimmed_eq_ignore_ascii_case};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum CellKind {
    #[serde(rename = "lut")]
    Lut,
    #[serde(rename = "ff")]
    Ff,
    #[serde(rename = "latch")]
    Latch,
    #[serde(rename = "constant")]
    Constant,
    #[serde(rename = "buffer")]
    Buffer,
    #[serde(rename = "iob")]
    Io,
    #[serde(rename = "gclk")]
    GlobalClockBuffer,
    #[serde(rename = "blockram")]
    BlockRam,
    #[serde(rename = "generic")]
    Generic,
    #[default]
    #[serde(rename = "unknown")]
    Unknown,
}

impl CellKind {
    pub fn classify(raw: &str) -> Self {
        let raw = raw.trim();
        if raw.is_empty() {
            return Self::Unknown;
        }
        if raw.eq_ignore_ascii_case("lut") {
            return Self::Lut;
        }
        if trimmed_contains_ignore_ascii_case(raw, "latch") {
            return Self::Latch;
        }
        if trimmed_eq_ignore_ascii_case(raw, "buffer") || trimmed_eq_ignore_ascii_case(raw, "buf") {
            return Self::Buffer;
        }
        if trimmed_eq_ignore_ascii_case(raw, "ff")
            || trimmed_contains_ignore_ascii_case(raw, "ff")
            || trimmed_contains_ignore_ascii_case(raw, "flipflop")
            || trimmed_contains_ignore_ascii_case(raw, "flip_flop")
            || trimmed_contains_ignore_ascii_case(raw, "dff")
        {
            return Self::Ff;
        }
        if raw.eq_ignore_ascii_case("constant") {
            return Self::Constant;
        }
        if raw.eq_ignore_ascii_case("iob") || raw.eq_ignore_ascii_case("io") {
            return Self::Io;
        }
        if raw.eq_ignore_ascii_case("gclk") || raw.eq_ignore_ascii_case("global_clock_buffer") {
            return Self::GlobalClockBuffer;
        }
        if trimmed_contains_ignore_ascii_case(raw, "blockram")
            || trimmed_contains_ignore_ascii_case(raw, "ramb")
        {
            return Self::BlockRam;
        }
        if raw.eq_ignore_ascii_case("generic") {
            return Self::Generic;
        }
        Self::Unknown
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lut => "lut",
            Self::Ff => "ff",
            Self::Latch => "latch",
            Self::Constant => "constant",
            Self::Buffer => "buffer",
            Self::Io => "iob",
            Self::GlobalClockBuffer => "gclk",
            Self::BlockRam => "blockram",
            Self::Generic => "generic",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for CellKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::classify(value))
    }
}

impl From<&str> for CellKind {
    fn from(value: &str) -> Self {
        Self::classify(value)
    }
}

impl From<String> for CellKind {
    fn from(value: String) -> Self {
        Self::classify(&value)
    }
}

impl fmt::Display for CellKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CellKind;

    #[test]
    fn classifies_common_cell_kinds() {
        assert_eq!(CellKind::classify("lut"), CellKind::Lut);
        assert_eq!(CellKind::classify("logic_ff"), CellKind::Ff);
        assert_eq!(CellKind::classify("BUFFER"), CellKind::Buffer);
        assert_eq!(CellKind::classify(""), CellKind::Unknown);
    }
}
