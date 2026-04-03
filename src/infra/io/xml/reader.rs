use super::{
    mapped_xml::load_fde_mapped_design_xml, physical_import::load_fde_physical_design_xml,
};
use anyhow::{Context, Result, bail};
use roxmltree::Document;

pub(super) fn load_design_xml(xml: &str) -> Result<crate::ir::Design> {
    let doc = Document::parse(xml).context("failed to parse design xml")?;
    let root = doc.root_element();
    if !root.has_tag_name("design") {
        bail!("root element is not <design>");
    }

    if is_physical_design_xml(root) {
        return load_fde_physical_design_xml(root);
    }
    load_fde_mapped_design_xml(root)
}

fn is_physical_design_xml(root: roxmltree::Node<'_, '_>) -> bool {
    root.descendants().any(|node| {
        node.has_tag_name("instance")
            && matches!(
                node.attribute("moduleRef"),
                Some("slice" | "iob" | "gclk" | "gclkiob")
            )
    })
}
