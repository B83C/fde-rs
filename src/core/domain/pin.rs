use super::{PrimitiveKind, SiteKind, ascii::trimmed_eq_ignore_ascii_case};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PinRole {
    LutInput(usize),
    LutOutput,
    RegisterOutput,
    RegisterClock,
    RegisterClockEnable,
    RegisterSetReset,
    RegisterData,
    SiteInput,
    SiteOutput,
    GlobalClockInput,
    GlobalClockOutput,
    GeneralOutput,
    Unknown,
}

impl PinRole {
    pub fn classify_for_primitive(primitive: PrimitiveKind, pin: &str) -> Self {
        if let Some(index) = primitive.lut_input_index(pin) {
            return Self::LutInput(index);
        }
        if primitive.is_lut_output_pin(pin) {
            return Self::LutOutput;
        }
        if primitive.is_register_output_pin(pin) {
            return Self::RegisterOutput;
        }
        if primitive.is_clock_pin(pin) {
            return Self::RegisterClock;
        }
        if primitive.is_clock_enable_pin(pin) {
            return Self::RegisterClockEnable;
        }
        if primitive.is_set_reset_pin(pin) {
            return Self::RegisterSetReset;
        }
        if primitive.is_register_data_pin(pin) {
            return Self::RegisterData;
        }
        if primitive.is_block_ram_output_pin(pin) {
            return Self::GeneralOutput;
        }
        Self::Unknown
    }

    pub fn classify_for_site(site_kind: SiteKind, pin: &str) -> Self {
        match site_kind {
            SiteKind::Iob if trimmed_eq_ignore_ascii_case(pin, "IN") => Self::SiteInput,
            SiteKind::Iob if trimmed_eq_ignore_ascii_case(pin, "OUT") => Self::SiteOutput,
            SiteKind::GclkIob if trimmed_eq_ignore_ascii_case(pin, "GCLKOUT") => {
                Self::GlobalClockOutput
            }
            SiteKind::Gclk if trimmed_eq_ignore_ascii_case(pin, "IN") => Self::GlobalClockInput,
            SiteKind::Gclk if trimmed_eq_ignore_ascii_case(pin, "OUT") => Self::GlobalClockOutput,
            SiteKind::LogicSlice
            | SiteKind::BlockRam
            | SiteKind::Const
            | SiteKind::Unplaced
            | SiteKind::Unknown => Self::Unknown,
            SiteKind::Iob | SiteKind::GclkIob | SiteKind::Gclk => Self::Unknown,
        }
    }

    pub fn classify_output_pin(primitive: PrimitiveKind, pin: &str) -> Self {
        let role = Self::classify_for_primitive(primitive, pin);
        if role.is_output_like() {
            return role;
        }
        if primitive.is_constant_source() {
            return Self::GeneralOutput;
        }
        if primitive.is_block_ram_output_pin(pin) {
            return Self::GeneralOutput;
        }
        if trimmed_eq_ignore_ascii_case(pin, "Q")
            || trimmed_eq_ignore_ascii_case(pin, "O")
            || trimmed_eq_ignore_ascii_case(pin, "Y")
            || trimmed_eq_ignore_ascii_case(pin, "OUT")
            || trimmed_eq_ignore_ascii_case(pin, "P")
            || trimmed_eq_ignore_ascii_case(pin, "G")
        {
            Self::GeneralOutput
        } else {
            Self::Unknown
        }
    }

    pub fn lut_input_index(self) -> Option<usize> {
        match self {
            Self::LutInput(index) => Some(index),
            _ => None,
        }
    }

    pub fn is_output_like(self) -> bool {
        matches!(
            self,
            Self::LutOutput | Self::RegisterOutput | Self::GlobalClockOutput | Self::GeneralOutput
        )
    }

    pub fn is_site_input(self) -> bool {
        matches!(self, Self::SiteInput)
    }

    pub fn is_site_output(self) -> bool {
        matches!(self, Self::SiteOutput)
    }

    pub fn is_global_clock_input(self) -> bool {
        matches!(self, Self::GlobalClockInput)
    }

    pub fn is_global_clock_output(self) -> bool {
        matches!(self, Self::GlobalClockOutput)
    }
}

#[cfg(test)]
mod tests {
    use super::PinRole;
    use crate::domain::{PrimitiveKind, SiteKind};

    #[test]
    fn classifies_primitive_and_site_pins() {
        let lut = PrimitiveKind::classify("lut", "LUT4");
        assert_eq!(
            PinRole::classify_for_primitive(lut, "ADR1"),
            PinRole::LutInput(1)
        );
        assert_eq!(
            PinRole::classify_for_primitive(lut, "O"),
            PinRole::LutOutput
        );

        let ff = PrimitiveKind::classify("ff", "DFF");
        assert_eq!(
            PinRole::classify_for_primitive(ff, "CLK"),
            PinRole::RegisterClock
        );
        assert_eq!(
            PinRole::classify_for_primitive(ff, "CKN"),
            PinRole::RegisterClock
        );
        assert_eq!(
            PinRole::classify_for_primitive(ff, "Q"),
            PinRole::RegisterOutput
        );
        assert_eq!(
            PinRole::classify_for_primitive(ff, "E"),
            PinRole::RegisterClockEnable
        );
        assert_eq!(
            PinRole::classify_for_primitive(ff, "RN"),
            PinRole::RegisterSetReset
        );
        assert_eq!(
            PinRole::classify_for_primitive(ff, "SN"),
            PinRole::RegisterSetReset
        );

        assert_eq!(
            PinRole::classify_for_site(SiteKind::Iob, "IN"),
            PinRole::SiteInput
        );
        assert_eq!(
            PinRole::classify_for_site(SiteKind::Gclk, "OUT"),
            PinRole::GlobalClockOutput
        );
        assert_eq!(
            PinRole::classify_output_pin(ff, " q "),
            PinRole::RegisterOutput
        );
    }
}
