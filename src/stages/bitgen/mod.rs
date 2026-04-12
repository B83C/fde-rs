mod api;
mod artifacts;
mod circuit;
mod config_image;
mod device;
mod frame_bitstream;
mod generator;
pub(crate) mod literal;
mod payload;
mod programming;
mod report;
mod sidecar;

#[cfg(test)]
mod tests;

use anyhow::Result;

pub use api::{BitgenOptions, run, run_with_reporter};
pub use config_image::{AppliedSiteConfig, ConfigImage, TileBitAssignment, TileConfigImage};
pub use device::{
    DeviceCell, DeviceDesign, DeviceEndpoint, DeviceNet, DevicePort, DeviceSinkGuide,
};
pub(crate) use device::{DeviceDesignIndex, DeviceEndpointRef};
pub use frame_bitstream::{SerializedTextBitstream, serialize_text_bitstream};

#[cfg(test)]
pub(crate) use programming::ProgrammedMemory;
#[cfg(test)]
pub(crate) use programming::RequestedConfig;
pub(crate) use programming::{ProgrammedSite, ProgrammingImage, build_programming_image};

pub fn build_config_image(
    device: &DeviceDesign,
    cil: &crate::cil::Cil,
    arch: Option<&crate::resource::Arch>,
    route_image: Option<&crate::route::DeviceRouteImage>,
) -> Result<ConfigImage> {
    let programming = build_programming_image(device, cil, route_image);
    config_image::encode_config_image(&programming, cil, arch)
}
