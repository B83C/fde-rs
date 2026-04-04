use crate::{
    edif::load_edif,
    io::{DesignInputFormat, detect_input_format, load_design},
    ir::Design,
    report::{StageOutput, StageReport},
};
use anyhow::{Result, bail};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub source_hint: Option<String>,
}

pub fn run_path(input: &Path, options: &ImportOptions) -> Result<StageOutput<Design>> {
    let mut design = match detect_input_format(input) {
        Some(DesignInputFormat::Edif) => load_edif(input)?,
        Some(DesignInputFormat::Xml | DesignInputFormat::Json) => load_design(input)?,
        Some(DesignInputFormat::Verilog) => {
            bail!(
                "Verilog import is intentionally unsupported in this Rust rewrite. Synthesize with Yosys first and pass the EDIF to `fde map` or `fde impl`."
            )
        }
        None => bail!("unsupported import format for {}", input.display()),
    };

    design.stage = "imported".to_string();
    design.note(format!("Imported from {}", input.display()));
    if let Some(hint) = &options.source_hint {
        design.note(format!("source_hint={hint}"));
    }

    let mut report = StageReport::new("import");
    report.push(format!(
        "Imported {} cells and {} nets from {}.",
        design.cells.len(),
        design.nets.len(),
        input.display()
    ));

    Ok(StageOutput {
        value: design,
        report,
    })
}
