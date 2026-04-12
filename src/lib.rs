mod app;
mod core;
mod infra;
mod stages;

pub use app::{cli, orchestrator, report};
pub use core::{domain, ir};
pub use infra::{cil, constraints, edif, io, resource};
pub(crate) use stages::analysis;
pub use stages::{bitgen, import, map, normalize, pack, place, route, sta};

pub use bitgen::{
    AppliedSiteConfig, ConfigImage, DeviceCell, DeviceDesign, DeviceEndpoint, DeviceNet,
    DevicePort, DeviceSinkGuide, SerializedTextBitstream, TileBitAssignment, TileConfigImage,
    build_config_image, serialize_text_bitstream,
};
pub use bitgen::{BitgenOptions, run as run_bitgen, run_with_reporter as run_bitgen_with_reporter};
pub(crate) use bitgen::{DeviceDesignIndex, DeviceEndpointRef};
pub use cil::{Cil, load_cil};
pub use constraints::{ConstraintEntry, load_constraints};
pub use domain::{
    CellKind, ClusterKind, ConstantKind, EndpointKind, NetOrigin, PinRole, PrimitiveKind, SiteKind,
    TimingPathCategory,
};
pub use import::{ImportOptions, run_path as run_import};
pub use ir::{
    BitstreamImage, Design, Placement, PlacementSite, RouteSegment, TimingGraph, TimingSummary,
};
pub use map::{
    MapOptions, load_input as load_map_input, run as run_map,
    run_with_reporter as run_map_with_reporter,
};
pub use normalize::{NormalizeOptions, run as run_normalize};
pub use orchestrator::{
    ImplementationOptions, run as run_implementation,
    run_with_reporter as run_implementation_with_reporter,
};
pub use pack::{PackOptions, run as run_pack, run_with_reporter as run_pack_with_reporter};
pub use place::{
    PlaceMode, PlaceOptions, run as run_place, run_with_reporter as run_place_with_reporter,
};
pub use report::{
    ImplementationReport, LineStageReporter, ReportStatus, StageEvent, StageLogLevel, StageOutput,
    StageReport, StageReporter, format_stage_event_line, format_stage_status_name,
};
pub use resource::{Arch, DelayModel, ResourceBundle, load_arch, load_delay_model};
pub use route::{
    DeviceRouteImage, DeviceRoutePip, RouteBit, RouteOptions, RoutedNetPip, load_route_pips,
    load_route_pips_xml, lower_design, materialize_route_image, route_device_design,
    route_device_design_with_reporter, run as run_route, run_with_artifacts_and_reporter,
    run_with_reporter as run_route_with_reporter,
};
pub use sta::{
    StaArtifact, StaError, StaOptions, run as run_sta, run_with_reporter as run_sta_with_reporter,
};
