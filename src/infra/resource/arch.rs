use anyhow::{Context, Result};
use roxmltree::Document;
use std::{collections::BTreeMap, fs, path::Path};

use crate::domain::SiteKind;

#[derive(Debug, Clone, Default)]
pub struct Pad {
    pub name: String,
    pub x: usize,
    pub y: usize,
    pub z: usize,
    pub tile_name: String,
    pub tile_type: String,
    pub site_kind: PadSiteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileKind {
    Logic,
    BlockRam,
    LeftIo,
    RightIo,
    TopIo,
    BottomIo,
    ClockTop,
    ClockBottom,
    ClockCenter,
    ClockVertical,
    ClockHorizontal,
    #[default]
    Unknown,
}

impl TileKind {
    pub fn classify(raw: &str) -> Self {
        let raw = raw.trim().to_ascii_uppercase();
        match raw.as_str() {
            "LEFT" => Self::LeftIo,
            "RIGHT" => Self::RightIo,
            "TOP" => Self::TopIo,
            "BOT" => Self::BottomIo,
            "BRAM" | "BLOCKRAM" | "BRAM16" => Self::BlockRam,
            "CLKT" => Self::ClockTop,
            "CLKB" => Self::ClockBottom,
            "CLKC" => Self::ClockCenter,
            "CLKV" => Self::ClockVertical,
            "CLKH" => Self::ClockHorizontal,
            "CENTER" => Self::Logic,
            _ if raw.starts_with("CENTER") => Self::Logic,
            _ if raw.starts_with("LBRAM") || raw.starts_with("RBRAM") => Self::BlockRam,
            _ => Self::Unknown,
        }
    }

    pub fn is_logic(self) -> bool {
        matches!(self, Self::Logic)
    }

    pub fn is_block_ram(self) -> bool {
        matches!(self, Self::BlockRam)
    }

    pub fn is_clock_pad_tile(self) -> bool {
        matches!(self, Self::ClockTop | Self::ClockBottom)
    }

    pub fn canonical_name(self) -> Option<&'static str> {
        match self {
            Self::Logic => Some("CENTER"),
            Self::BlockRam => None,
            Self::LeftIo => Some("LEFT"),
            Self::RightIo => Some("RIGHT"),
            Self::TopIo => Some("TOP"),
            Self::BottomIo => Some("BOT"),
            Self::ClockTop => Some("CLKT"),
            Self::ClockBottom => Some("CLKB"),
            Self::ClockCenter => Some("CLKC"),
            Self::ClockVertical => Some("CLKV"),
            Self::ClockHorizontal => Some("CLKH"),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PadSiteKind {
    #[default]
    Iob,
    GclkIob,
}

impl PadSiteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iob => "IOB",
            Self::GclkIob => "GCLKIOB",
        }
    }

    pub fn site_kind(self) -> SiteKind {
        match self {
            Self::Iob => SiteKind::Iob,
            Self::GclkIob => SiteKind::GclkIob,
        }
    }

    pub fn io_type_name(self) -> &'static str {
        match self {
            Self::Iob => "IOB",
            Self::GclkIob => "GCLKIOB",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TileInstance {
    pub name: String,
    pub tile_type: String,
    pub logic_x: usize,
    pub logic_y: usize,
    pub bit_x: usize,
    pub bit_y: usize,
    pub phy_x: usize,
    pub phy_y: usize,
}

impl Pad {
    pub fn tile_kind(&self) -> TileKind {
        TileKind::classify(&self.tile_type)
    }
}

impl TileInstance {
    pub fn kind(&self) -> TileKind {
        TileKind::classify(&self.tile_type)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TileSideCapacity {
    pub left: usize,
    pub right: usize,
    pub bottom: usize,
    pub top: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Arch {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub slices_per_tile: usize,
    pub lut_inputs: usize,
    pub wire_r: f64,
    pub wire_c: f64,
    pub pads: Vec<Pad>,
    pub pad_lookup: BTreeMap<String, (usize, usize)>,
    pub pad_sites: BTreeMap<String, Pad>,
    pub tiles: BTreeMap<(usize, usize), TileInstance>,
    pub tile_side_capacities: BTreeMap<String, TileSideCapacity>,
    pub default_horizontal_capacity: usize,
    pub default_vertical_capacity: usize,
}

pub fn load_arch(path: &Path) -> Result<Arch> {
    let xml = fs::read_to_string(path)
        .with_context(|| format!("failed to read architecture file {}", path.display()))?;
    let doc = Document::parse(&xml)
        .with_context(|| format!("failed to parse architecture file {}", path.display()))?;
    let root = doc.root_element();
    let mut arch = Arch {
        name: root
            .attribute("name")
            .or_else(|| root.attribute("device"))
            .unwrap_or("device")
            .to_string(),
        ..Arch::default()
    };

    populate_device_info(&mut arch, root);
    populate_tile_side_capacities(&mut arch, root);
    populate_tile_instances(&mut arch, root);
    populate_pads(&mut arch, root);
    apply_arch_defaults(&mut arch);

    Ok(arch)
}

impl Arch {
    pub fn tile_at(&self, x: usize, y: usize) -> Option<&TileInstance> {
        self.tiles.get(&(x, y))
    }

    pub fn pad(&self, name: &str) -> Option<&Pad> {
        self.pad_sites.get(name)
    }

    pub fn pad_at_site(
        &self,
        x: usize,
        y: usize,
        z: Option<usize>,
        kind: Option<PadSiteKind>,
    ) -> Option<&Pad> {
        self.pads.iter().find(|pad| {
            pad.x == x
                && pad.y == y
                && z.is_none_or(|z| pad.z == z)
                && kind.is_none_or(|kind| pad.site_kind == kind)
        })
    }

    pub fn logic_sites(&self) -> Vec<(usize, usize)> {
        if !self.tiles.is_empty() {
            let mut sites = self
                .tiles
                .values()
                .filter(|tile| tile.kind().is_logic())
                .map(|tile| (tile.logic_x, tile.logic_y))
                .collect::<Vec<_>>();
            sites.sort_unstable();
            sites.dedup();
            if !sites.is_empty() {
                return sites;
            }
        }
        if self.width <= 2 || self.height <= 2 {
            return (0..self.width)
                .flat_map(|x| (0..self.height).map(move |y| (x, y)))
                .collect();
        }
        (1..self.width - 1)
            .flat_map(|x| (1..self.height - 1).map(move |y| (x, y)))
            .collect()
    }

    pub fn block_ram_sites(&self) -> Vec<(usize, usize)> {
        if self.tiles.is_empty() {
            return Vec::new();
        }

        let mut sites = self
            .tiles
            .values()
            .filter(|tile| is_block_ram_site_type(&tile.tile_type))
            .map(|tile| (tile.logic_x, tile.logic_y))
            .collect::<Vec<_>>();
        sites.sort_unstable();
        sites.dedup();
        sites
    }

    pub fn fallback_port_position(&self, index: usize, input: bool) -> (usize, usize) {
        if !self.pads.is_empty() {
            let pad = &self.pads[index % self.pads.len()];
            return (pad.x, pad.y);
        }
        if input {
            (0, (index + 1).min(self.height.saturating_sub(1)))
        } else {
            (
                self.width.saturating_sub(1),
                (index + 1).min(self.height.saturating_sub(1)),
            )
        }
    }

    pub fn edge_capacity(&self, lhs: (usize, usize), rhs: (usize, usize)) -> usize {
        if lhs.0.abs_diff(rhs.0) + lhs.1.abs_diff(rhs.1) != 1 {
            return 1;
        }

        if lhs.1 == rhs.1 {
            let (left, right) = if lhs.0 <= rhs.0 {
                (lhs, rhs)
            } else {
                (rhs, lhs)
            };
            return resolve_edge_capacity(
                self.side_capacity(left, TileSide::Right),
                self.side_capacity(right, TileSide::Left),
                self.default_horizontal_capacity,
            );
        }

        let (bottom, top) = if lhs.1 <= rhs.1 {
            (lhs, rhs)
        } else {
            (rhs, lhs)
        };
        resolve_edge_capacity(
            self.side_capacity(bottom, TileSide::Top),
            self.side_capacity(top, TileSide::Bottom),
            self.default_vertical_capacity,
        )
    }

    fn side_capacity(&self, point: (usize, usize), side: TileSide) -> usize {
        self.tile_at(point.0, point.1)
            .and_then(|tile| self.tile_side_capacities.get(&tile.tile_type))
            .map(|capacity| capacity.side(side))
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy)]
enum TileSide {
    Left,
    Right,
    Bottom,
    Top,
}

impl TileSideCapacity {
    fn side(self, side: TileSide) -> usize {
        match side {
            TileSide::Left => self.left,
            TileSide::Right => self.right,
            TileSide::Bottom => self.bottom,
            TileSide::Top => self.top,
        }
    }
}

fn populate_device_info(arch: &mut Arch, root: roxmltree::Node<'_, '_>) {
    let Some(device_info) = root
        .descendants()
        .find(|node| node.has_tag_name("device_info"))
    else {
        return;
    };

    if let Some((width, height)) = device_info.attribute("scale").and_then(parse_point) {
        arch.width = width;
        arch.height = height;
    }
    arch.slices_per_tile = device_info
        .attribute("slice_per_tile")
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    arch.lut_inputs = device_info
        .attribute("LUT_Inputs")
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    if let Some(wire_timing) = device_info.attribute("wire_timming") {
        let mut parts = wire_timing.split(',').map(str::trim);
        arch.wire_r = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.04);
        arch.wire_c = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.03);
    }
}

fn populate_tile_side_capacities(arch: &mut Arch, root: roxmltree::Node<'_, '_>) {
    for cell in root
        .descendants()
        .filter(|node| node.has_tag_name("cell") && node.attribute("type") == Some("TILE"))
    {
        let Some(name) = cell.attribute("name") else {
            continue;
        };
        let mut capacity = TileSideCapacity::default();
        for port in cell.children().filter(|node| node.has_tag_name("port")) {
            let Some(width) = side_port_width(port) else {
                continue;
            };
            match port.attribute("side").unwrap_or_default() {
                "left" => capacity.left = capacity.left.max(width),
                "right" => capacity.right = capacity.right.max(width),
                "bottom" => capacity.bottom = capacity.bottom.max(width),
                "top" => capacity.top = capacity.top.max(width),
                _ => {}
            }
        }
        if capacity.left > 0 || capacity.right > 0 || capacity.bottom > 0 || capacity.top > 0 {
            arch.tile_side_capacities.insert(name.to_string(), capacity);
        }
    }
}

fn populate_tile_instances(arch: &mut Arch, root: roxmltree::Node<'_, '_>) {
    for tile in root.descendants().filter(|node| {
        node.has_tag_name("instance") && node.attribute("libraryRef") == Some("tile")
    }) {
        let Some((logic_x, logic_y)) = tile.attribute("logic_pos").and_then(parse_point) else {
            continue;
        };
        let (bit_x, bit_y) = tile
            .attribute("bit_pos")
            .and_then(parse_point)
            .unwrap_or((logic_x, logic_y));
        let (phy_x, phy_y) = tile
            .attribute("phy_pos")
            .and_then(parse_point)
            .unwrap_or((logic_x, logic_y));
        let instance = TileInstance {
            name: tile.attribute("name").unwrap_or_default().to_string(),
            tile_type: tile.attribute("cellRef").unwrap_or_default().to_string(),
            logic_x,
            logic_y,
            bit_x,
            bit_y,
            phy_x,
            phy_y,
        };
        arch.tiles.insert((logic_x, logic_y), instance);
    }
}

fn populate_pads(arch: &mut Arch, root: roxmltree::Node<'_, '_>) {
    for pad in root.descendants().filter(|node| node.has_tag_name("pad")) {
        let name = pad.attribute("name").unwrap_or_default().to_string();
        if name.is_empty() {
            continue;
        }
        let raw_pos = pad.attribute("pos").or_else(|| pad.attribute("position"));
        let Some((x, y, z)) = raw_pos.and_then(parse_point3) else {
            continue;
        };
        let (tile_name, tile_type, site_kind) = arch
            .tiles
            .get(&(x, y))
            .map(|tile| {
                let site_kind = if tile.kind().is_clock_pad_tile() {
                    PadSiteKind::GclkIob
                } else {
                    PadSiteKind::Iob
                };
                (tile.name.clone(), tile.tile_type.clone(), site_kind)
            })
            .unwrap_or_default();
        arch.pad_lookup.insert(name.clone(), (x, y));
        let pad = Pad {
            name: name.clone(),
            x,
            y,
            z,
            tile_name,
            tile_type,
            site_kind,
        };
        arch.pad_sites.insert(name.clone(), pad.clone());
        arch.pads.push(pad);
    }
}

fn apply_arch_defaults(arch: &mut Arch) {
    if arch.width == 0 || arch.height == 0 {
        arch.width = 32;
        arch.height = 32;
    }
    if arch.slices_per_tile == 0 {
        arch.slices_per_tile = 2;
    }
    if arch.lut_inputs == 0 {
        arch.lut_inputs = 4;
    }
    if arch.wire_r == 0.0 {
        arch.wire_r = 0.04;
    }
    if arch.wire_c == 0.0 {
        arch.wire_c = 0.03;
    }
    if arch.default_horizontal_capacity == 0 {
        arch.default_horizontal_capacity = arch
            .tile_side_capacities
            .values()
            .flat_map(|capacity| [capacity.left, capacity.right])
            .filter(|capacity| *capacity > 0)
            .max()
            .unwrap_or(1);
    }
    if arch.default_vertical_capacity == 0 {
        arch.default_vertical_capacity = arch
            .tile_side_capacities
            .values()
            .flat_map(|capacity| [capacity.bottom, capacity.top])
            .filter(|capacity| *capacity > 0)
            .max()
            .unwrap_or(1);
    }
}

fn is_block_ram_site_type(tile_type: &str) -> bool {
    let raw = tile_type.trim().to_ascii_uppercase();
    matches!(raw.as_str(), "BRAM" | "BLOCKRAM" | "BRAM16")
        || ((raw.starts_with("LBRAM") || raw.starts_with("RBRAM")) && raw.ends_with('D'))
}

fn parse_point(raw: &str) -> Option<(usize, usize)> {
    let mut parts = raw.split(',').map(str::trim);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    Some((x, y))
}

fn parse_point3(raw: &str) -> Option<(usize, usize, usize)> {
    let mut parts = raw.split(',').map(str::trim);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some((x, y, z))
}

fn side_port_width(node: roxmltree::Node<'_, '_>) -> Option<usize> {
    let msb = node.attribute("msb")?.parse::<usize>().ok()?;
    let lsb = node.attribute("lsb").unwrap_or("0").parse::<usize>().ok()?;
    Some(msb.abs_diff(lsb).saturating_add(1))
}

fn resolve_edge_capacity(lhs: usize, rhs: usize, fallback: usize) -> usize {
    let fallback = fallback.max(1);
    match (lhs, rhs) {
        (0, 0) => fallback,
        (0, value) | (value, 0) => value.max(1),
        (lhs, rhs) => lhs.min(rhs).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::{Arch, TileInstance, TileKind};
    use std::collections::BTreeMap;

    #[test]
    fn logic_sites_prefer_center_tile_instances_when_available() {
        let mut arch = Arch {
            width: 4,
            height: 4,
            tiles: BTreeMap::new(),
            ..Arch::default()
        };
        arch.tiles.insert(
            (0, 0),
            TileInstance {
                tile_type: "LEFT".to_string(),
                logic_x: 0,
                logic_y: 0,
                ..TileInstance::default()
            },
        );
        arch.tiles.insert(
            (1, 1),
            TileInstance {
                tile_type: "CENTER".to_string(),
                logic_x: 1,
                logic_y: 1,
                ..TileInstance::default()
            },
        );
        arch.tiles.insert(
            (2, 2),
            TileInstance {
                tile_type: "CLKH".to_string(),
                logic_x: 2,
                logic_y: 2,
                ..TileInstance::default()
            },
        );

        assert_eq!(arch.logic_sites(), vec![(1, 1)]);
        assert_eq!(arch.tiles[&(1, 1)].kind(), TileKind::Logic);
    }
}
