use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    bitgen::BitgenOptions,
    cil::load_cil,
    constraints::{SharedConstraints, load_constraints},
    ir::Design,
    resource::load_arch,
    route::{lower_design, materialize_design_route_image},
};

pub(crate) struct PreparedBitgen {
    pub(crate) options: BitgenOptions,
}

pub(crate) fn load_constraints_or_empty(path: Option<&PathBuf>) -> Result<SharedConstraints> {
    match path {
        Some(path) => load_constraints(path).map(Arc::<[_]>::from),
        None => Ok(Arc::from([])),
    }
}

pub(crate) fn default_sidecar_path(output: &Path) -> PathBuf {
    output.with_extension("bit.txt")
}

pub(crate) fn prepare_bitgen(
    design: &Design,
    arch_path: Option<&PathBuf>,
    cil_path: Option<&PathBuf>,
) -> Result<PreparedBitgen> {
    let arch = match arch_path {
        Some(path) => Some(load_arch(path)?),
        None => None,
    };
    let cil = match cil_path {
        Some(path) => Some(load_cil(path)?),
        None => None,
    };
    let arch_name = arch.as_ref().map(|arch| arch.name.clone());
    let device_design = match (arch.as_ref(), cil.as_ref()) {
        (Some(arch), Some(cil)) => Some(lower_design(design.clone(), arch, Some(cil), &[])?),
        _ => None,
    };
    let route_image = match (arch.as_ref(), arch_path, cil.as_ref()) {
        (Some(arch), Some(arch_path), Some(cil)) => {
            materialize_design_route_image(design, arch, arch_path, cil)?
        }
        _ => None,
    };
    Ok(PreparedBitgen {
        options: BitgenOptions {
            arch_name,
            arch_path: arch_path.cloned(),
            cil_path: cil_path.cloned(),
            cil,
            device_design,
            route_image,
        },
    })
}
