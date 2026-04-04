use anyhow::{Result, anyhow};
use roxmltree::Node;

pub(super) fn attr(node: &Node<'_, '_>, name: &str) -> String {
    node.attribute(name).unwrap_or_default().to_string()
}

pub(super) fn expand_bus_port_names(node: Node<'_, '_>) -> Vec<String> {
    let name = attr(&node, "name");
    let Some(msb) = node
        .attribute("msb")
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return vec![name];
    };
    let lsb = node
        .attribute("lsb")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(msb);
    if msb >= lsb {
        (lsb..=msb)
            .rev()
            .map(|index| format!("{name}[{index}]"))
            .collect()
    } else {
        (msb..=lsb)
            .map(|index| format!("{name}[{index}]"))
            .collect()
    }
}

pub(super) fn top_module_node<'a, 'input>(root: Node<'a, 'input>) -> Result<Node<'a, 'input>> {
    let top_module_ref = root
        .children()
        .find(|node| node.has_tag_name("topModule"))
        .ok_or_else(|| anyhow!("missing <topModule> section"))?;
    let library_name = attr(&top_module_ref, "libraryRef");
    let module_name = attr(&top_module_ref, "name");
    root.children()
        .find(|node| node.has_tag_name("library") && attr(node, "name") == library_name)
        .and_then(|library| {
            library
                .children()
                .find(|node| node.has_tag_name("module") && attr(node, "name") == module_name)
        })
        .ok_or_else(|| anyhow!("FDE top module {module_name} not found"))
}

pub(super) fn parse_point(value: &str) -> Option<(usize, usize, usize)> {
    let mut fields = value.split(',').map(str::trim);
    let x = fields.next()?.parse::<usize>().ok()?;
    let y = fields.next()?.parse::<usize>().ok()?;
    let z = fields.next().unwrap_or("0").parse::<usize>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some((x, y, z))
}

#[cfg(test)]
mod tests {
    use roxmltree::Document;

    use super::{expand_bus_port_names, parse_point, top_module_node};

    #[test]
    fn expands_bus_port_names_for_descending_and_ascending_ranges() {
        let descending = Document::parse(r#"<port name="led" msb="3" lsb="0"/>"#).expect("xml");
        assert_eq!(
            expand_bus_port_names(descending.root_element()),
            vec!["led[3]", "led[2]", "led[1]", "led[0]"]
        );

        let ascending = Document::parse(r#"<port name="led" msb="0" lsb="3"/>"#).expect("xml");
        assert_eq!(
            expand_bus_port_names(ascending.root_element()),
            vec!["led[0]", "led[1]", "led[2]", "led[3]"]
        );
    }

    #[test]
    fn parses_points_and_rejects_extra_coordinates() {
        assert_eq!(parse_point("1,2"), Some((1, 2, 0)));
        assert_eq!(parse_point("1, 2, 3"), Some((1, 2, 3)));
        assert_eq!(parse_point("1,2,3,4"), None);
    }

    #[test]
    fn resolves_top_module_from_design_root() {
        let doc = Document::parse(
            r#"
<design name="top">
  <topModule libraryRef="work_lib" name="demo"/>
  <library name="work_lib">
    <module name="demo" type="GENERIC"/>
  </library>
</design>
"#,
        )
        .expect("xml");
        let module = top_module_node(doc.root_element()).expect("top module");
        assert_eq!(module.tag_name().name(), "module");
        assert_eq!(module.attribute("name"), Some("demo"));
    }
}
