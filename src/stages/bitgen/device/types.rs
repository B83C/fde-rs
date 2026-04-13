use crate::{
    domain::{CellKind, EndpointKind, NetOrigin, PrimitiveKind, SequentialInitValue, SiteKind},
    ir::{PortDirection, Property},
    resource::TileKind,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceDesign {
    pub name: String,
    pub device: String,
    pub ports: Vec<DevicePort>,
    pub cells: Vec<DeviceCell>,
    pub nets: Vec<DeviceNet>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevicePort {
    pub port_name: String,
    pub direction: PortDirection,
    pub pin_name: String,
    pub site_kind: SiteKind,
    pub site_name: String,
    pub tile_name: String,
    pub tile_type: String,
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

impl DevicePort {
    pub fn new(
        port_name: impl Into<String>,
        direction: PortDirection,
        pin_name: impl Into<String>,
    ) -> Self {
        Self {
            port_name: port_name.into(),
            direction,
            pin_name: pin_name.into(),
            ..Self::default()
        }
    }

    pub fn sited(
        mut self,
        site_kind: SiteKind,
        site_name: impl Into<String>,
        tile_name: impl Into<String>,
        tile_type: impl Into<String>,
        position: (usize, usize, usize),
    ) -> Self {
        self.site_kind = site_kind;
        self.site_name = site_name.into();
        self.tile_name = tile_name.into();
        self.tile_type = tile_type.into();
        self.x = position.0;
        self.y = position.1;
        self.z = position.2;
        self
    }

    pub fn site_kind_class(&self) -> SiteKind {
        self.site_kind
    }

    pub fn tile_kind(&self) -> TileKind {
        TileKind::classify(&self.tile_type)
    }

    pub fn tile_wire_prefix(&self) -> &str {
        self.tile_kind()
            .canonical_name()
            .unwrap_or(self.tile_type.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceCell {
    pub cell_name: String,
    #[serde(default)]
    pub kind: CellKind,
    pub type_name: String,
    #[serde(default)]
    pub properties: Vec<Property>,
    pub site_kind: SiteKind,
    pub site_name: String,
    pub bel: String,
    pub tile_name: String,
    pub tile_type: String,
    pub x: usize,
    pub y: usize,
    pub z: usize,
    pub cluster_name: Option<String>,
    pub synthetic: bool,
}

impl DeviceCell {
    pub fn new(cell_name: impl Into<String>, kind: CellKind, type_name: impl Into<String>) -> Self {
        Self {
            cell_name: cell_name.into(),
            kind,
            type_name: type_name.into(),
            ..Self::default()
        }
    }

    pub fn with_properties(mut self, properties: Vec<Property>) -> Self {
        self.properties = properties;
        self
    }

    pub fn placed(
        mut self,
        site_kind: SiteKind,
        site_name: impl Into<String>,
        bel: impl Into<String>,
        tile_name: impl Into<String>,
        tile_type: impl Into<String>,
        position: (usize, usize, usize),
    ) -> Self {
        self.site_kind = site_kind;
        self.site_name = site_name.into();
        self.bel = bel.into();
        self.tile_name = tile_name.into();
        self.tile_type = tile_type.into();
        self.x = position.0;
        self.y = position.1;
        self.z = position.2;
        self
    }

    pub fn in_cluster(mut self, cluster_name: impl Into<String>) -> Self {
        self.cluster_name = Some(cluster_name.into());
        self
    }

    pub fn synthetic(mut self) -> Self {
        self.synthetic = true;
        self
    }

    pub fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::from_cell_kind(self.kind, &self.type_name)
    }

    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|property| property.key.eq_ignore_ascii_case(key))
            .map(|property| property.value.as_str())
    }

    pub fn register_init_value(&self) -> Option<SequentialInitValue> {
        self.primitive_kind()
            .is_sequential()
            .then(|| {
                SequentialInitValue::from_explicit_or_type_name(
                    self.property("init"),
                    &self.type_name,
                )
            })
            .flatten()
    }

    pub fn site_kind_class(&self) -> SiteKind {
        self.site_kind
    }

    pub fn tile_kind(&self) -> TileKind {
        TileKind::classify(&self.tile_type)
    }

    pub fn tile_wire_prefix(&self) -> &str {
        self.tile_kind()
            .canonical_name()
            .unwrap_or(self.tile_type.as_str())
    }

    pub fn site_slot(&self) -> usize {
        trailing_index(&self.site_name).unwrap_or(self.z)
    }

    pub fn is_sited(&self) -> bool {
        !self.tile_type.is_empty() && !self.site_name.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceNet {
    pub name: String,
    pub driver: Option<DeviceEndpoint>,
    pub sinks: Vec<DeviceEndpoint>,
    pub origin: NetOrigin,
    #[serde(default)]
    pub guide_tiles: Vec<(usize, usize)>,
    #[serde(default)]
    pub sink_guides: Vec<DeviceSinkGuide>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceSinkGuide {
    pub sink: DeviceEndpoint,
    #[serde(default)]
    pub tiles: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEndpoint {
    pub kind: EndpointKind,
    pub name: String,
    pub pin: String,
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

impl DeviceNet {
    pub fn new(name: impl Into<String>, origin: NetOrigin) -> Self {
        Self {
            name: name.into(),
            origin,
            ..Self::default()
        }
    }

    pub fn with_driver(mut self, driver: DeviceEndpoint) -> Self {
        self.driver = Some(driver);
        self
    }

    pub fn with_sink(mut self, sink: DeviceEndpoint) -> Self {
        self.sinks.push(sink);
        self
    }

    pub fn with_guide_tiles(mut self, guide_tiles: Vec<(usize, usize)>) -> Self {
        self.guide_tiles = guide_tiles;
        self
    }

    pub fn with_sink_guides(mut self, sink_guides: Vec<DeviceSinkGuide>) -> Self {
        self.sink_guides = sink_guides;
        self
    }

    pub fn origin_kind(&self) -> NetOrigin {
        self.origin
    }

    pub fn guide_tiles_for_sink<'a>(&'a self, sink: &DeviceEndpoint) -> &'a [(usize, usize)] {
        self.sink_guides
            .iter()
            .find(|guide| guide.sink == *sink && !guide.tiles.is_empty())
            .map(|guide| guide.tiles.as_slice())
            .unwrap_or(self.guide_tiles.as_slice())
    }
}

impl DeviceEndpoint {
    pub fn new(
        kind: EndpointKind,
        name: impl Into<String>,
        pin: impl Into<String>,
        position: (usize, usize, usize),
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            pin: pin.into(),
            x: position.0,
            y: position.1,
            z: position.2,
        }
    }

    pub fn cell(
        name: impl Into<String>,
        pin: impl Into<String>,
        position: (usize, usize, usize),
    ) -> Self {
        Self::new(EndpointKind::Cell, name, pin, position)
    }

    pub fn port(
        name: impl Into<String>,
        pin: impl Into<String>,
        position: (usize, usize, usize),
    ) -> Self {
        Self::new(EndpointKind::Port, name, pin, position)
    }

    pub fn endpoint_kind(&self) -> EndpointKind {
        self.kind
    }

    pub fn is_cell(&self) -> bool {
        self.endpoint_kind().is_cell()
    }

    pub fn is_port(&self) -> bool {
        self.endpoint_kind().is_port()
    }
}

fn trailing_index(raw: &str) -> Option<usize> {
    raw.chars()
        .rev()
        .find(|ch| ch.is_ascii_digit())
        .and_then(|ch| ch.to_digit(10))
        .map(|digit| digit as usize)
}

#[cfg(test)]
mod tests {
    use super::DeviceCell;
    use crate::{domain::CellKind, domain::SequentialInitValue};

    #[test]
    fn register_init_value_falls_back_to_cpp_ff_type_defaults() {
        let ff_low = DeviceCell::new("ff_low", CellKind::Ff, "DFFRHQ");
        let ff_high = DeviceCell::new("ff_high", CellKind::Ff, "DFFSHQ");

        assert_eq!(ff_low.register_init_value(), Some(SequentialInitValue::Low));
        assert_eq!(
            ff_high.register_init_value(),
            Some(SequentialInitValue::High)
        );
    }
}
