use super::ascii::trimmed_eq_ignore_ascii_case;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum ClusterKind {
    #[serde(rename = "logic")]
    Logic,
    #[serde(rename = "blockram")]
    BlockRam,
    #[default]
    #[serde(rename = "unknown")]
    Unknown,
}

impl ClusterKind {
    pub fn classify(raw: &str) -> Self {
        if trimmed_eq_ignore_ascii_case(raw, "logic") {
            Self::Logic
        } else if trimmed_eq_ignore_ascii_case(raw, "blockram")
            || trimmed_eq_ignore_ascii_case(raw, "bram")
        {
            Self::BlockRam
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Logic => "logic",
            Self::BlockRam => "blockram",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for ClusterKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::classify(value))
    }
}

impl From<&str> for ClusterKind {
    fn from(value: &str) -> Self {
        Self::classify(value)
    }
}

impl From<String> for ClusterKind {
    fn from(value: String) -> Self {
        Self::classify(&value)
    }
}

impl fmt::Display for ClusterKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ClusterKind;

    #[test]
    fn classifies_logic_cluster_kind_case_insensitively() {
        assert_eq!(ClusterKind::classify("logic"), ClusterKind::Logic);
        assert_eq!(ClusterKind::classify("LOGIC"), ClusterKind::Logic);
        assert_eq!(ClusterKind::classify("BRAM"), ClusterKind::BlockRam);
        assert_eq!(ClusterKind::classify("other"), ClusterKind::Unknown);
    }
}
