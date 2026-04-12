mod modern;
mod options;
mod report;
mod resources;

use crate::report::ImplementationReport;
use anyhow::Result;

pub use options::ImplementationOptions;

pub fn run(options: &ImplementationOptions) -> Result<ImplementationReport> {
    modern::run(options)
}

pub fn run_with_reporter(
    options: &ImplementationOptions,
    reporter: &mut dyn crate::report::StageReporter,
) -> Result<ImplementationReport> {
    modern::run_with_reporter(options, reporter)
}
