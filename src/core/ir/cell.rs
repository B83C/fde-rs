use crate::domain::{
    CellKind, ConstantKind, PrimitiveKind, SequentialCellType, SequentialInitValue,
};
use serde::{Deserialize, Serialize};

use super::{CellPin, Property};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SliceBindingKind {
    #[default]
    Lut,
    Sequential,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceBinding {
    pub slot: usize,
    pub kind: SliceBindingKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cell {
    pub name: String,
    pub kind: CellKind,
    pub type_name: String,
    #[serde(default)]
    pub inputs: Vec<CellPin>,
    #[serde(default)]
    pub outputs: Vec<CellPin>,
    #[serde(default)]
    pub properties: Vec<Property>,
    #[serde(default)]
    pub cluster: Option<String>,
    #[serde(default)]
    pub slice_binding: Option<SliceBinding>,
}

impl Cell {
    pub fn new(name: impl Into<String>, kind: CellKind, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            type_name: type_name.into(),
            ..Self::default()
        }
    }

    pub fn lut(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self::new(name, CellKind::Lut, type_name)
    }

    pub fn ff(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self::new(name, CellKind::Ff, type_name)
    }

    pub fn with_input(mut self, port: impl Into<String>, net: impl Into<String>) -> Self {
        self.inputs.push(CellPin::new(port, net));
        self
    }

    pub fn with_output(mut self, port: impl Into<String>, net: impl Into<String>) -> Self {
        self.outputs.push(CellPin::new(port, net));
        self
    }

    pub fn in_cluster(mut self, cluster: impl Into<String>) -> Self {
        self.cluster = Some(cluster.into());
        self
    }

    pub fn with_slice_binding(mut self, slot: usize, kind: SliceBindingKind) -> Self {
        self.slice_binding = Some(SliceBinding { slot, kind });
        self
    }

    pub fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::from_cell_kind(self.kind, &self.type_name)
    }

    pub fn constant_kind(&self) -> Option<ConstantKind> {
        self.primitive_kind().constant_kind()
    }

    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|prop| prop.key.eq_ignore_ascii_case(key))
            .map(|prop| prop.value.as_str())
    }

    pub fn set_property(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(existing) = self
            .properties
            .iter_mut()
            .find(|prop| prop.key.eq_ignore_ascii_case(&key))
        {
            existing.key = key;
            existing.value = value;
        } else {
            self.properties.push(Property::new(key, value));
        }
    }

    pub fn set_slice_binding(&mut self, slot: usize, kind: SliceBindingKind) {
        self.slice_binding = Some(SliceBinding { slot, kind });
    }

    pub fn is_sequential(&self) -> bool {
        self.primitive_kind().is_sequential()
    }

    pub fn is_lut(&self) -> bool {
        self.primitive_kind().is_lut()
    }

    pub fn is_constant_source(&self) -> bool {
        self.primitive_kind().is_constant_source()
    }

    pub fn is_buffer(&self) -> bool {
        self.primitive_kind().is_buffer()
    }

    pub fn is_block_ram(&self) -> bool {
        self.primitive_kind().is_block_ram()
    }

    pub fn register_clock_net(&self) -> Option<&str> {
        self.input_net_matching(|primitive, port| primitive.is_clock_pin(port))
    }

    pub fn register_clock_enable_net(&self) -> Option<&str> {
        self.input_net_matching(|primitive, port| primitive.is_clock_enable_pin(port))
    }

    pub fn register_set_reset_net(&self) -> Option<&str> {
        self.input_net_matching(|primitive, port| primitive.is_set_reset_pin(port))
    }

    pub fn register_init_value(&self) -> Option<SequentialInitValue> {
        self.is_sequential()
            .then(|| {
                SequentialInitValue::from_explicit_or_type_name(
                    self.property("init"),
                    &self.type_name,
                )
            })
            .flatten()
    }

    pub fn register_clock_is_inverted(&self) -> bool {
        self.is_sequential()
            && (self.inputs.iter().any(|pin| {
                pin.port.eq_ignore_ascii_case("CKN") || pin.port.eq_ignore_ascii_case("CLKN")
            }) || SequentialCellType::from_type_name(&self.type_name)
                .is_some_and(SequentialCellType::clock_is_inverted_by_default))
    }

    fn input_net_matching(
        &self,
        mut predicate: impl FnMut(PrimitiveKind, &str) -> bool,
    ) -> Option<&str> {
        let primitive = self.primitive_kind();
        self.inputs
            .iter()
            .find(|pin| predicate(primitive, &pin.port))
            .map(|pin| pin.net.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Cell;

    #[test]
    fn register_control_helpers_cover_common_ff_pins() {
        let ff = Cell::ff("ff0", "EDFFNRHQ")
            .with_input("CKN", "clk")
            .with_input("E", "ce")
            .with_input("RN", "rst");

        assert_eq!(ff.register_clock_net(), Some("clk"));
        assert!(ff.register_clock_is_inverted());
        assert_eq!(ff.register_clock_enable_net(), Some("ce"));
        assert_eq!(ff.register_set_reset_net(), Some("rst"));
    }

    #[test]
    fn register_init_value_reads_single_bit_init_property() {
        let mut ff = Cell::ff("ff0", "DFFHQ");
        ff.set_property("INIT", "1");

        assert_eq!(
            ff.register_init_value(),
            Some(super::SequentialInitValue::High)
        );
    }

    #[test]
    fn register_init_value_falls_back_to_cpp_ff_type_defaults() {
        let ff_low = Cell::ff("ff_low", "DFFRHQ");
        let ff_high = Cell::ff("ff_high", "DFFSHQ");

        assert_eq!(
            ff_low.register_init_value(),
            Some(super::SequentialInitValue::Low)
        );
        assert_eq!(
            ff_high.register_init_value(),
            Some(super::SequentialInitValue::High)
        );
    }
}
