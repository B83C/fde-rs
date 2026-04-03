use std::collections::BTreeSet;

use super::{CellId, ClusterId, Design, DesignIndex, SliceBindingKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignedClusterCellKind {
    Lut,
    Sequential,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignedClusterCell {
    pub cell_id: CellId,
    pub slot: usize,
    pub kind: AssignedClusterCellKind,
}

pub fn assign_cluster_slice_cells(
    design: &Design,
    index: &DesignIndex<'_>,
    cluster_id: ClusterId,
) -> Vec<AssignedClusterCell> {
    authoritative_cluster_assignments(design, index, cluster_id)
        .unwrap_or_else(|| heuristic_cluster_assignments(design, index, cluster_id))
}

fn authoritative_cluster_assignments(
    design: &Design,
    index: &DesignIndex<'_>,
    cluster_id: ClusterId,
) -> Option<Vec<AssignedClusterCell>> {
    let mut assigned = Vec::new();
    let mut used = BTreeSet::new();

    for &cell_id in index.cluster_members(cluster_id) {
        let cell = index.cell(design, cell_id);
        let Some(kind) = assigned_kind(cell) else {
            continue;
        };
        let binding = cell.slice_binding?;
        let expected_kind = match kind {
            AssignedClusterCellKind::Lut => SliceBindingKind::Lut,
            AssignedClusterCellKind::Sequential => SliceBindingKind::Sequential,
            AssignedClusterCellKind::Other => return None,
        };
        if binding.kind != expected_kind {
            return None;
        }
        assigned.push(AssignedClusterCell {
            cell_id,
            slot: binding.slot,
            kind,
        });
        used.insert(cell_id);
    }

    let mut next_slot = assigned
        .iter()
        .map(|cell| cell.slot)
        .max()
        .map_or(0, |slot| slot + 1);
    for &cell_id in index.cluster_members(cluster_id) {
        if used.contains(&cell_id) {
            continue;
        }
        assigned.push(AssignedClusterCell {
            cell_id,
            slot: next_slot,
            kind: assigned_kind(index.cell(design, cell_id))
                .unwrap_or(AssignedClusterCellKind::Other),
        });
        next_slot += 1;
    }

    sort_assigned_cells(&mut assigned);
    Some(assigned)
}

fn heuristic_cluster_assignments(
    design: &Design,
    index: &DesignIndex<'_>,
    cluster_id: ClusterId,
) -> Vec<AssignedClusterCell> {
    let mut assigned = Vec::new();
    let mut used = BTreeSet::<CellId>::new();
    let mut paired = Vec::<(Option<CellId>, CellId)>::new();

    for &cell_id in index.cluster_members(cluster_id) {
        let cell = index.cell(design, cell_id);
        if !cell.is_sequential() || used.contains(&cell_id) {
            continue;
        }
        let driver_id = lut_feeding_ff(design, index, cell_id, cluster_id)
            .filter(|driver_id| used.insert(*driver_id));
        if used.insert(cell_id) {
            paired.push((driver_id, cell_id));
        }
    }

    let paired_count = paired.len();
    for (pair_index, (driver_id, ff_id)) in paired.into_iter().enumerate() {
        let slot = pair_index;
        if let Some(driver_id) = driver_id {
            assigned.push(AssignedClusterCell {
                cell_id: driver_id,
                slot,
                kind: AssignedClusterCellKind::Lut,
            });
        }
        assigned.push(AssignedClusterCell {
            cell_id: ff_id,
            slot,
            kind: AssignedClusterCellKind::Sequential,
        });
    }

    let mut slot = paired_count;
    for &cell_id in index.cluster_members(cluster_id) {
        if used.contains(&cell_id) {
            continue;
        }
        assigned.push(AssignedClusterCell {
            cell_id,
            slot,
            kind: assigned_kind(index.cell(design, cell_id))
                .unwrap_or(AssignedClusterCellKind::Other),
        });
        used.insert(cell_id);
        slot += 1;
    }

    assigned
}

fn assigned_kind(cell: &crate::ir::Cell) -> Option<AssignedClusterCellKind> {
    if cell.is_lut() {
        Some(AssignedClusterCellKind::Lut)
    } else if cell.is_sequential() {
        Some(AssignedClusterCellKind::Sequential)
    } else {
        None
    }
}

fn lut_feeding_ff(
    design: &Design,
    index: &DesignIndex<'_>,
    ff_id: CellId,
    cluster_id: ClusterId,
) -> Option<CellId> {
    let ff = index.cell(design, ff_id);
    let d_net = ff
        .inputs
        .iter()
        .find(|pin| pin.port.eq_ignore_ascii_case("D"))?
        .net
        .as_str();
    index
        .cluster_members(cluster_id)
        .iter()
        .find_map(|cell_id| {
            let cell = index.cell(design, *cell_id);
            if !cell.is_lut() {
                return None;
            }
            cell.outputs
                .iter()
                .any(|pin| pin.net == d_net)
                .then_some(*cell_id)
        })
}

fn sort_assigned_cells(assigned: &mut [AssignedClusterCell]) {
    assigned.sort_by(|lhs, rhs| {
        (lhs.slot, kind_sort_key(lhs.kind), lhs.cell_id.index()).cmp(&(
            rhs.slot,
            kind_sort_key(rhs.kind),
            rhs.cell_id.index(),
        ))
    });
}

fn kind_sort_key(kind: AssignedClusterCellKind) -> usize {
    match kind {
        AssignedClusterCellKind::Lut => 0,
        AssignedClusterCellKind::Sequential => 1,
        AssignedClusterCellKind::Other => 2,
    }
}
