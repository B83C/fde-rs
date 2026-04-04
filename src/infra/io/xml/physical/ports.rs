use crate::{
    ir::{Design, RoutePip},
    resource::PadSiteKind,
};

use super::super::writer::{
    GCLKIOB_DEFAULT_CONFIGS, IOB_DEFAULT_CONFIGS, PortInstanceBinding, XmlWriteContext,
    default_config_map, default_configs, is_clock_input_port, ordered_configs,
};

pub(super) fn build_port_bindings(
    design: &Design,
    context: &XmlWriteContext<'_>,
) -> Vec<PortInstanceBinding> {
    let mut clock_port_index = 0usize;
    design
        .ports
        .iter()
        .map(|port| {
            let clock_input = is_clock_input_port(design, &port.name);
            let pin_name = port.pin.as_deref().or_else(|| {
                context
                    ._constraints
                    .iter()
                    .find(|constraint| constraint.port_name == port.name)
                    .map(|constraint| constraint.pin_name.as_str())
            });
            let pad = context
                .arch
                .and_then(|arch| pin_name.and_then(|pin| arch.pad(pin)));
            let pad_module_ref = match pad.map(|pad| pad.site_kind) {
                Some(PadSiteKind::GclkIob) => "gclkiob",
                Some(PadSiteKind::Iob) => "iob",
                None if clock_input => "gclkiob",
                None => "iob",
            };
            let fallback_position = port.x.zip(port.y).map(|(x, y)| (x, y, port.z.unwrap_or(0)));
            let pad_position = pad.map(|pad| (pad.x, pad.y, pad.z)).or(fallback_position);
            let gclk_instance_name = if clock_input && pad_module_ref == "gclkiob" {
                let name = format!("iGclk_buf__{clock_port_index}__");
                clock_port_index += 1;
                Some(name)
            } else {
                None
            };
            let tile_wire_prefix = pad.map(|pad| {
                pad.tile_kind()
                    .canonical_name()
                    .unwrap_or(pad.tile_type.as_str())
                    .to_string()
            });
            let tile_position = pad.map(|pad| (pad.x, pad.y, pad.z)).or(fallback_position);
            PortInstanceBinding {
                port_name: port.name.clone(),
                direction: port.direction.clone(),
                pad_instance_name: port.name.clone(),
                pad_module_ref,
                pad_position,
                gclk_instance_name,
                gclk_position: pad_position,
                clock_input,
                tile_wire_prefix,
                tile_position,
            }
        })
        .collect()
}

pub(super) fn build_pad_configs(binding: &PortInstanceBinding) -> Vec<(String, String)> {
    match binding.pad_module_ref {
        "gclkiob" => default_configs(GCLKIOB_DEFAULT_CONFIGS),
        "iob" => {
            let mut configs = default_config_map(IOB_DEFAULT_CONFIGS);
            if binding.direction.is_input_like() {
                configs.insert("IMUX".to_string(), "1".to_string());
            }
            if binding.direction.is_output_like() {
                configs.insert("OMUX".to_string(), "O".to_string());
                configs.insert("OUTMUX".to_string(), "1".to_string());
                configs.insert("DRIVEATTRBOX".to_string(), "12".to_string());
                configs.insert("SLEW".to_string(), "SLOW".to_string());
            }
            ordered_configs(IOB_DEFAULT_CONFIGS, configs)
        }
        _ => Vec::new(),
    }
}

fn gclk_input_pips(binding: &PortInstanceBinding) -> Vec<RoutePip> {
    let Some(prefix) = binding.tile_wire_prefix.as_deref() else {
        return Vec::new();
    };
    let Some((x, y, z)) = binding.tile_position else {
        return Vec::new();
    };
    vec![RoutePip::new(
        (x, y),
        format!("{prefix}_CLKPAD{z}"),
        format!("{prefix}_GCLKBUF{z}_IN"),
    )]
}

pub(super) fn split_clock_route_pips(
    route_pips: &[RoutePip],
    binding: &PortInstanceBinding,
) -> (Vec<RoutePip>, Vec<RoutePip>) {
    let helper_pip = gclk_input_pips(binding).into_iter().next();
    let Some(helper_pip) = helper_pip else {
        return (route_pips.to_vec(), Vec::new());
    };

    let mut logical_pips = Vec::with_capacity(route_pips.len());
    let mut helper_pips = Vec::new();
    for pip in route_pips {
        if *pip == helper_pip {
            helper_pips.push(pip.clone());
        } else {
            logical_pips.push(pip.clone());
        }
    }
    (logical_pips, helper_pips)
}

#[cfg(test)]
mod tests {
    use super::build_port_bindings;
    use crate::ir::{Design, Port};

    #[test]
    fn fallback_port_binding_preserves_existing_site_slot() {
        let design = Design {
            stage: "routed".to_string(),
            ports: vec![Port::output("led").at_site(5, 1, 2)],
            ..Design::default()
        };

        let bindings = build_port_bindings(&design, &Default::default());
        let binding = bindings.first().expect("binding");

        assert_eq!(binding.pad_position, Some((5, 1, 2)));
        assert_eq!(binding.tile_position, Some((5, 1, 2)));
    }
}
