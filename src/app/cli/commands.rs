use anyhow::{Context, Result};
use std::{fs, sync::Arc};

use crate::{
    bitgen,
    cil::load_cil,
    import::{self, ImportOptions},
    io::{DesignWriteContext, load_design, save_design, save_design_with_context},
    map::{self, MapOptions},
    normalize::{self, NormalizeOptions},
    orchestrator,
    pack::{self, PackOptions},
    place::{self, PlaceOptions},
    report::{LineStageReporter, print_stage_report, run_stage_with_reporter},
    resource::{load_arch, load_delay_model},
    route::{self, RouteOptions},
    sta::{self, StaOptions},
};

use super::{
    args::{
        BitgenArgs, Command, ImplArgs, ImportArgs, MapArgs, NormalizeArgs, PackArgs, PlaceArgs,
        RouteArgs, StaArgs,
    },
    helpers::{default_sidecar_path, load_constraints_or_empty, prepare_bitgen},
};

pub(crate) fn dispatch_command(command: Command) -> Result<()> {
    match command {
        Command::Map(args) => run_map(args, true),
        Command::Pack(args) => run_pack(args, true),
        Command::Place(args) => run_place(args, true),
        Command::Route(args) => run_route(args, true),
        Command::Sta(args) => run_sta(args, true),
        Command::Bitgen(args) => run_bitgen(args, true),
        Command::Normalize(args) => run_normalize(args, true),
        Command::Import(args) => run_import(args, true),
        Command::Impl(args) => run_impl(*args),
    }
}

pub(crate) fn run_map(args: MapArgs, emit_report: bool) -> Result<()> {
    let design = map::load_input(&args.input)?;
    let options = MapOptions {
        lut_size: args.lut_size,
        cell_library: args.cell_library.clone(),
        emit_structural_verilog: args.verilog_output.is_some(),
    };
    let result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "map",
            &mut reporter,
            || map::run(design.clone(), &options),
            |reporter| map::run_with_reporter(design.clone(), &options, reporter),
        )?
    } else {
        map::run(design, &options)?
    };
    save_design(&result.value.design, &args.output)?;
    if let Some(path) = args.verilog_output
        && let Some(verilog) = result.value.structural_verilog
    {
        fs::write(&path, verilog).with_context(|| format!("failed to write {}", path.display()))?;
    }
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_pack(args: PackArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let options = PackOptions {
        family: args.family,
        capacity: args.capacity,
        cell_library: args.cell_library,
        dcp_library: args.dcp_library,
        config: args.config,
    };
    let result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "pack",
            &mut reporter,
            || pack::run(design.clone(), &options),
            |reporter| pack::run_with_reporter(design.clone(), &options, reporter),
        )?
    } else {
        pack::run(design, &options)?
    };
    save_design(&result.value, &args.output)?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_place(args: PlaceArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let arch = Arc::new(load_arch(&args.arch)?);
    let delay = load_delay_model(args.delay.as_deref())?;
    let constraints = load_constraints_or_empty(args.constraints.as_ref())?;
    let options = PlaceOptions {
        arch: Arc::clone(&arch),
        delay: delay.map(Arc::new),
        constraints: Arc::clone(&constraints),
        mode: args.mode.into(),
        seed: args.seed,
    };
    let result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "place",
            &mut reporter,
            || place::run(design.clone(), &options),
            |reporter| place::run_with_reporter(design.clone(), &options, reporter),
        )?
    } else {
        place::run(design, &options)?
    };
    save_design_with_context(
        &result.value,
        &args.output,
        &DesignWriteContext {
            arch: Some(arch.as_ref()),
            constraints: constraints.as_ref(),
            ..DesignWriteContext::default()
        },
    )?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_route(args: RouteArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let arch = Arc::new(load_arch(&args.arch)?);
    let constraints = load_constraints_or_empty(args.constraints.as_ref())?;
    let cil = match args.cil.as_ref() {
        Some(path) => Some(load_cil(path)?),
        None => None,
    };
    let device_design = cil
        .as_ref()
        .map(|cil| {
            route::lower_design(
                design.clone(),
                arch.as_ref(),
                Some(cil),
                constraints.as_ref(),
            )
        })
        .transpose()?;
    let options = RouteOptions {
        arch: Arc::clone(&arch),
        arch_path: args.arch.clone(),
        constraints: Arc::clone(&constraints),
        cil: cil.clone(),
        device_design,
    };
    let result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "route",
            &mut reporter,
            || route::run(design.clone(), &options),
            |reporter| route::run_with_reporter(design.clone(), &options, reporter),
        )?
    } else {
        route::run(design, &options)?
    };
    save_design_with_context(
        &result.value,
        &args.output,
        &DesignWriteContext {
            arch: Some(arch.as_ref()),
            cil: cil.as_ref(),
            constraints: constraints.as_ref(),
            cil_path: args.cil.as_deref(),
        },
    )?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_sta(args: StaArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let arch = match args.arch.as_ref() {
        Some(path) => Some(load_arch(path)?),
        None => None,
    };
    let delay = load_delay_model(args.delay.as_deref())?;
    let options = StaOptions {
        arch: arch.clone().map(Arc::new),
        delay: delay.map(Arc::new),
    };
    let mut result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "sta",
            &mut reporter,
            || sta::run(design.clone(), &options),
            |reporter| sta::run_with_reporter(design.clone(), &options, reporter),
        )?
    } else {
        sta::run(design, &options)?
    };
    if let Some(path) = args.timing_library.as_ref() {
        result
            .report
            .push(format!("Referenced timing library {}", path.display()));
    }
    save_design_with_context(
        &result.value.design,
        &args.output,
        &DesignWriteContext {
            arch: arch.as_ref(),
            ..DesignWriteContext::default()
        },
    )?;
    fs::write(&args.report, &result.value.report_text)
        .with_context(|| format!("failed to write {}", args.report.display()))?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_bitgen(args: BitgenArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let prepared = prepare_bitgen(&design, args.arch.as_ref(), args.cil.as_ref())?;
    let result = if emit_report {
        let mut stdout_logger = |line: String| print!("{line}");
        let mut cli_reporter = LineStageReporter::cli(&mut stdout_logger);
        let mut reporter = Some(&mut cli_reporter as &mut dyn crate::report::StageReporter);
        run_stage_with_reporter(
            "bitgen",
            &mut reporter,
            || bitgen::run(design.clone(), &prepared.options),
            |reporter| bitgen::run_with_reporter(design.clone(), &prepared.options, reporter),
        )?
    } else {
        bitgen::run(design, &prepared.options)?
    };
    fs::write(&args.output, &result.value.bytes)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    if args.emit_sidecar || args.sidecar.is_some() {
        let sidecar = args
            .sidecar
            .unwrap_or_else(|| default_sidecar_path(&args.output));
        fs::write(&sidecar, &result.value.sidecar_text)
            .with_context(|| format!("failed to write {}", sidecar.display()))?;
    }
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_normalize(args: NormalizeArgs, emit_report: bool) -> Result<()> {
    let design = load_design(&args.input)?;
    let result = normalize::run(
        design,
        &NormalizeOptions {
            cell_library: args.cell_library,
            config: args.config,
        },
    )?;
    save_design(&result.value, &args.output)?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_import(args: ImportArgs, emit_report: bool) -> Result<()> {
    let result = import::run_path(&args.input, &ImportOptions::default())?;
    save_design(&result.value, &args.output)?;
    if emit_report {
        print_stage_report(&result.report);
    }
    Ok(())
}

pub(crate) fn run_impl(args: ImplArgs) -> Result<()> {
    let mut stdout_logger = |line: String| print!("{line}");
    let mut reporter = LineStageReporter::cli(&mut stdout_logger);
    let report = orchestrator::run_with_reporter(&args.into(), &mut reporter)?;
    for stage in &report.stages {
        print_stage_report(stage);
    }
    if let Some(summary_path) = report.artifacts.get("summary") {
        println!("[impl] Wrote summary to {}", summary_path);
    }
    if let Some(log_path) = report.artifacts.get("log") {
        println!("[impl] Wrote log to {}", log_path);
    }
    if let Some(report_path) = report.artifacts.get("report") {
        println!("[impl] Wrote report to {}", report_path);
    }
    Ok(())
}
