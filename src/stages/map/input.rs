use crate::{
    edif::load_edif,
    io::{DesignInputFormat, detect_input_format, load_design},
    ir::Design,
};
use anyhow::{Result, bail};
use std::path::Path;

pub fn load_input(path: &Path) -> Result<Design> {
    match detect_input_format(path) {
        Some(DesignInputFormat::Edif) => load_edif(path),
        Some(DesignInputFormat::Xml | DesignInputFormat::Json) => load_design(path),
        Some(DesignInputFormat::Verilog) => bail!(
            "Verilog is not a primary frontend in this rewrite. Use Yosys to generate EDIF first."
        ),
        None => bail!("unsupported map input format for {}", path.display()),
    }
}
