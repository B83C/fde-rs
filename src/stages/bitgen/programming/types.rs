use crate::{
    bitgen::DeviceCell,
    domain::{SequentialInitValue, SiteKind},
    route::DeviceRoutePip,
};

#[derive(Debug, Clone)]
pub(crate) struct SiteInstance {
    pub(crate) tile_name: String,
    pub(crate) tile_type: String,
    pub(crate) site_kind: SiteKind,
    pub(crate) site_name: String,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) z: usize,
    pub(crate) cells: Vec<DeviceCell>,
}

#[derive(Debug, Clone)]
pub(crate) struct SiteProgram {
    pub(crate) tile_name: String,
    pub(crate) tile_type: String,
    pub(crate) site_kind: SiteKind,
    pub(crate) site_name: String,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) kind: SiteProgramKind,
}

#[derive(Debug, Clone)]
pub(crate) enum SiteProgramKind {
    LogicSlice(SliceProgram),
    BlockRam(BlockRamProgram),
    Iob(IobProgram),
    Gclk,
    GclkIob,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SliceProgram {
    pub(crate) slots: [SliceSlotProgram; 2],
    pub(crate) clock_enable_mode: SliceClockEnableMode,
}

impl SliceProgram {
    pub(crate) fn has_sequential(&self) -> bool {
        self.slots.iter().any(|slot| slot.ff.is_some())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SliceSlotProgram {
    pub(crate) lut: Option<LutProgram>,
    pub(crate) ff: Option<SequentialProgram>,
}

#[derive(Debug, Clone)]
pub(crate) struct LutProgram {
    pub(crate) truth_table_bits: Vec<u8>,
    pub(crate) output_usage: SliceLutOutputUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SliceLutOutputUsage {
    HiddenLocalOnly,
    RoutedOutput,
}

#[derive(Debug, Clone)]
pub(crate) struct SequentialProgram {
    pub(crate) init: SequentialInitValue,
    pub(crate) data_path: SliceFfDataPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SliceFfDataPath {
    LocalLut,
    SiteBypass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SliceClockEnableMode {
    #[default]
    None,
    DirectCe,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IobProgram {
    pub(crate) input_used: bool,
    pub(crate) output_used: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockRamProgram {
    pub(crate) port_a_attr: Option<String>,
    pub(crate) port_b_attr: Option<String>,
    pub(crate) clka_used: bool,
    pub(crate) clkb_used: bool,
    pub(crate) ena_used: bool,
    pub(crate) enb_used: bool,
    pub(crate) wea_used: bool,
    pub(crate) web_used: bool,
    pub(crate) rsta_used: bool,
    pub(crate) rstb_used: bool,
    pub(crate) init_words: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestedConfig {
    pub(crate) cfg_name: String,
    pub(crate) function_name: String,
}

#[cfg(test)]
impl RequestedConfig {
    pub(crate) fn new(cfg_name: impl Into<String>, function_name: impl Into<String>) -> Self {
        Self {
            cfg_name: cfg_name.into(),
            function_name: function_name.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProgrammingImage {
    pub(crate) sites: Vec<ProgrammedSite>,
    pub(crate) memories: Vec<ProgrammedMemory>,
    pub(crate) routes: Vec<DeviceRoutePip>,
    pub(crate) notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgrammedSite {
    pub(crate) tile_name: String,
    pub(crate) tile_type: String,
    pub(crate) site_kind: SiteKind,
    pub(crate) site_name: String,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) requests: Vec<RequestedConfig>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgrammedMemory {
    pub(crate) tile_name: String,
    pub(crate) init_words: Vec<(String, String)>,
}

#[cfg(test)]
impl ProgrammedSite {
    pub(crate) fn new(
        tile_name: impl Into<String>,
        tile_type: impl Into<String>,
        site_kind: SiteKind,
        site_name: impl Into<String>,
        x: usize,
        y: usize,
        requests: Vec<RequestedConfig>,
    ) -> Self {
        Self {
            tile_name: tile_name.into(),
            tile_type: tile_type.into(),
            site_kind,
            site_name: site_name.into(),
            x,
            y,
            requests,
        }
    }
}

#[cfg(test)]
impl ProgrammedMemory {
    pub(crate) fn new(tile_name: impl Into<String>, init_words: Vec<(String, String)>) -> Self {
        Self {
            tile_name: tile_name.into(),
            init_words,
        }
    }
}
