use super::{DeviceCell, DeviceLowering};
use crate::{
    cil::Cil,
    domain::{ClusterKind, SiteKind},
    ir::{
        AssignedClusterCellKind, CellId, ClusterId, Design, DesignIndex, assign_cluster_slice_cells,
    },
    resource::Arch,
};
use std::collections::BTreeSet;

impl<'a> DeviceLowering<'a> {
    pub(super) fn materialize_cells(&mut self) {
        let lowered = lower_original_cells(self.design, &self.index, self.arch, self.cil);
        let mut seen_names = self
            .device
            .cells
            .iter()
            .map(|cell| cell.cell_name.clone())
            .collect::<BTreeSet<_>>();
        for (cell_id, cell) in lowered {
            if !seen_names.insert(cell.cell_name.clone()) {
                continue;
            }
            self.push_original_cell(cell_id, cell);
        }
    }
}

fn lower_original_cells(
    design: &Design,
    index: &DesignIndex<'_>,
    arch: &Arch,
    cil: Option<&Cil>,
) -> Vec<(CellId, DeviceCell)> {
    let mut lowered = Vec::new();
    for cluster_index in 0..design.clusters.len() {
        let cluster_id = ClusterId::new(cluster_index);
        let cluster = index.cluster(design, cluster_id);
        let x = cluster.x.unwrap_or(0);
        let y = cluster.y.unwrap_or(0);
        let z = cluster.z.unwrap_or(0);
        let tile = arch.tile_at(x, y);
        let tile_name = tile.map(|tile| tile.name.clone()).unwrap_or_default();
        let tile_type = tile
            .map(|tile| tile.tile_type.clone())
            .unwrap_or_else(|| "CENTER".to_string());
        let (site_kind, site_name, bels) = match cluster.kind {
            ClusterKind::Logic => (
                SiteKind::LogicSlice,
                cil.and_then(|cil| cil.site_name_for_kind(&tile_type, SiteKind::LogicSlice, z))
                    .unwrap_or("SLICE")
                    .to_string(),
                assign_cluster_bels(design, index, cluster_id),
            ),
            ClusterKind::BlockRam => (
                SiteKind::BlockRam,
                cil.and_then(|cil| cil.site_name_for_kind(&tile_type, SiteKind::BlockRam, z))
                    .unwrap_or("BRAM")
                    .to_string(),
                index
                    .cluster_members(cluster_id)
                    .iter()
                    .copied()
                    .map(|cell_id| (cell_id, "BRAM".to_string()))
                    .collect::<Vec<_>>(),
            ),
            ClusterKind::Unknown => (
                SiteKind::Unplaced,
                String::new(),
                index
                    .cluster_members(cluster_id)
                    .iter()
                    .copied()
                    .map(|cell_id| (cell_id, "BEL".to_string()))
                    .collect::<Vec<_>>(),
            ),
        };
        for (cell_id, bel) in bels {
            let cell = index.cell(design, cell_id);
            lowered.push((
                cell_id,
                DeviceCell::new(cell.name.clone(), cell.kind, cell.type_name.clone())
                    .with_properties(cell.properties.clone())
                    .placed(
                        site_kind,
                        site_name.clone(),
                        bel,
                        tile_name.clone(),
                        tile_type.clone(),
                        (x, y, z),
                    )
                    .in_cluster(cluster.name.clone()),
            ));
        }
    }

    for (cell_index, cell) in design
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.cluster.is_none())
    {
        let (site_kind, site_name, bel) = if cell.is_constant_source() {
            (SiteKind::Const, cell.type_name.clone(), "DRV".to_string())
        } else {
            (
                SiteKind::Unplaced,
                cell.type_name.clone(),
                "BEL".to_string(),
            )
        };
        lowered.push((
            CellId::new(cell_index),
            DeviceCell::new(cell.name.clone(), cell.kind, cell.type_name.clone())
                .with_properties(cell.properties.clone())
                .placed(
                    site_kind,
                    site_name,
                    bel,
                    String::new(),
                    String::new(),
                    (0, 0, 0),
                ),
        ));
    }

    lowered
}

fn assign_cluster_bels(
    design: &Design,
    index: &DesignIndex<'_>,
    cluster_id: ClusterId,
) -> Vec<(crate::ir::CellId, String)> {
    assign_cluster_slice_cells(design, index, cluster_id)
        .into_iter()
        .map(|assignment| {
            let bel = match assignment.kind {
                AssignedClusterCellKind::Lut => format!("LUT{}", assignment.slot),
                AssignedClusterCellKind::Sequential => format!("FF{}", assignment.slot),
                AssignedClusterCellKind::Other => format!("BEL{}", assignment.slot),
            };
            (assignment.cell_id, bel)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::assign_cluster_bels;
    use crate::ir::DesignIndex;
    use crate::ir::{Cell, CellKind, Cluster, Design, Endpoint, Net, SliceBindingKind};

    fn lut(name: &str, init: &str, net: &str) -> Cell {
        let mut cell = Cell::new(name, CellKind::Lut, "LUT2").with_output("O", net);
        cell.set_property("lut_init", init);
        cell
    }

    fn ff(name: &str, d_net: &str, q_net: &str) -> Cell {
        Cell::new(name, CellKind::Ff, "DFFHQ")
            .with_input("D", d_net)
            .with_output("Q", q_net)
    }

    #[test]
    fn dual_lut_ff_pairs_preserve_cluster_pair_order() {
        let design = Design {
            cells: vec![
                lut("lut_a", "0xA", "d_a"),
                ff("ff_a", "d_a", "q_a"),
                lut("lut_c", "0xC", "d_c"),
                ff("ff_c", "d_c", "q_c"),
            ],
            nets: vec![
                Net::new("d_a")
                    .with_driver(Endpoint::cell("lut_a", "O"))
                    .with_sink(Endpoint::cell("ff_a", "D")),
                Net::new("d_c")
                    .with_driver(Endpoint::cell("lut_c", "O"))
                    .with_sink(Endpoint::cell("ff_c", "D")),
            ],
            clusters: vec![Cluster::logic("clb_0000").with_members(vec![
                "lut_a".to_string(),
                "ff_a".to_string(),
                "lut_c".to_string(),
                "ff_c".to_string(),
            ])],
            ..Design::default()
        };
        let index = DesignIndex::build(&design);
        let assigned = assign_cluster_bels(&design, &index, crate::ir::ClusterId::new(0));

        assert_eq!(
            assigned,
            vec![
                (crate::ir::CellId::new(0), "LUT0".to_string()),
                (crate::ir::CellId::new(1), "FF0".to_string()),
                (crate::ir::CellId::new(2), "LUT1".to_string()),
                (crate::ir::CellId::new(3), "FF1".to_string()),
            ]
        );
    }

    #[test]
    fn single_lut_ff_pair_stays_in_slot_zero() {
        let design = Design {
            cells: vec![lut("lut0", "0xF", "d"), ff("ff0", "d", "q")],
            nets: vec![
                Net::new("d")
                    .with_driver(Endpoint::cell("lut0", "O"))
                    .with_sink(Endpoint::cell("ff0", "D")),
            ],
            clusters: vec![
                Cluster::logic("clb_0000")
                    .with_members(vec!["lut0".to_string(), "ff0".to_string()]),
            ],
            ..Design::default()
        };
        let index = DesignIndex::build(&design);
        let assigned = assign_cluster_bels(&design, &index, crate::ir::ClusterId::new(0));

        assert_eq!(
            assigned,
            vec![
                (crate::ir::CellId::new(0), "LUT0".to_string()),
                (crate::ir::CellId::new(1), "FF0".to_string()),
            ]
        );
    }

    #[test]
    fn authoritative_slice_bindings_override_heuristic_slot_order() {
        let design = Design {
            cells: vec![
                lut("lut_a", "0xA", "d_a").with_slice_binding(0, SliceBindingKind::Lut),
                ff("ff_a", "d_a", "q_a").with_slice_binding(0, SliceBindingKind::Sequential),
                lut("lut_b", "0xC", "d_b").with_slice_binding(1, SliceBindingKind::Lut),
                ff("ff_b", "d_b", "q_b").with_slice_binding(1, SliceBindingKind::Sequential),
            ],
            nets: vec![
                Net::new("d_a")
                    .with_driver(Endpoint::cell("lut_a", "O"))
                    .with_sink(Endpoint::cell("ff_a", "D")),
                Net::new("d_b")
                    .with_driver(Endpoint::cell("lut_b", "O"))
                    .with_sink(Endpoint::cell("ff_b", "D")),
            ],
            clusters: vec![Cluster::logic("clb_0000").with_members(vec![
                "lut_a".to_string(),
                "ff_a".to_string(),
                "lut_b".to_string(),
                "ff_b".to_string(),
            ])],
            ..Design::default()
        };
        let index = DesignIndex::build(&design);
        let assigned = assign_cluster_bels(&design, &index, crate::ir::ClusterId::new(0));

        assert_eq!(
            assigned,
            vec![
                (crate::ir::CellId::new(0), "LUT0".to_string()),
                (crate::ir::CellId::new(1), "FF0".to_string()),
                (crate::ir::CellId::new(2), "LUT1".to_string()),
                (crate::ir::CellId::new(3), "FF1".to_string()),
            ]
        );
    }
}
