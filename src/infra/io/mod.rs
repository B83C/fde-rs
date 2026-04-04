mod json;
mod xml;

use crate::{cil::Cil, constraints::ConstraintEntry, ir::Design, resource::Arch};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignInputFormat {
    Edif,
    Xml,
    Json,
    Verilog,
}

impl DesignInputFormat {
    pub fn detect(path: &Path) -> Option<Self> {
        match file_extension(path).as_str() {
            "edf" | "edif" => Some(Self::Edif),
            "xml" => Some(Self::Xml),
            "json" => Some(Self::Json),
            "v" | "sv" => Some(Self::Verilog),
            _ => None,
        }
    }

    pub fn is_serialized_design(self) -> bool {
        matches!(self, Self::Xml | Self::Json)
    }
}

pub fn detect_input_format(path: &Path) -> Option<DesignInputFormat> {
    DesignInputFormat::detect(path)
}

pub fn load_design(path: &Path) -> Result<Design> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read design {}", path.display()))?;
    match detect_input_format(path) {
        Some(DesignInputFormat::Json) => json::load_design_json(&text, path),
        Some(DesignInputFormat::Xml) => xml::load_design_xml(&text),
        Some(_) | None => bail!(
            "unsupported serialized design format for {}",
            path.display()
        ),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DesignWriteContext<'a> {
    pub arch: Option<&'a Arch>,
    pub cil: Option<&'a Cil>,
    pub constraints: &'a [ConstraintEntry],
    pub cil_path: Option<&'a Path>,
}

pub fn save_design(design: &Design, path: &Path) -> Result<()> {
    save_design_with_context(design, path, &DesignWriteContext::default())
}

pub fn save_design_with_context(
    design: &Design,
    path: &Path,
    context: &DesignWriteContext<'_>,
) -> Result<()> {
    let data = match file_extension(path).as_str() {
        "json" => json::save_design_json(design)?,
        _ => xml::save_fde_design_xml_with_context(design, context)?,
    };
    fs::write(path, data).with_context(|| format!("failed to write design {}", path.display()))
}

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DesignInputFormat, detect_input_format};

    #[test]
    fn detects_supported_input_formats_by_extension() {
        assert_eq!(
            detect_input_format(Path::new("design.edf")),
            Some(DesignInputFormat::Edif)
        );
        assert_eq!(
            detect_input_format(Path::new("design.xml")),
            Some(DesignInputFormat::Xml)
        );
        assert_eq!(
            detect_input_format(Path::new("design.json")),
            Some(DesignInputFormat::Json)
        );
        assert_eq!(
            detect_input_format(Path::new("design.sv")),
            Some(DesignInputFormat::Verilog)
        );
        assert_eq!(detect_input_format(Path::new("design.txt")), None);
    }
}
