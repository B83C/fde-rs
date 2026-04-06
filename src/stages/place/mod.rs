mod cost;
mod graph;
mod model;
mod solver;
mod support;

use crate::{
    analysis::annotate_net_criticality,
    constraints::{apply_constraints, ensure_port_positions},
    domain::ClusterKind,
    ir::Design,
    report::{StageOutput, StageReport},
    resource::{Arch, SharedArch, SharedDelayModel},
};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use self::model::Point;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlaceMode {
    BoundingBox,
    TimingDriven,
}

pub const DEFAULT_PLACE_SEED: u64 = 1;

#[derive(Debug, Clone)]
pub struct PlaceOptions {
    pub arch: SharedArch,
    pub delay: Option<SharedDelayModel>,
    pub constraints: crate::constraints::SharedConstraints,
    pub mode: PlaceMode,
    pub seed: u64,
}

pub fn run(mut design: Design, options: &PlaceOptions) -> Result<StageOutput<Design>> {
    if matches!(options.mode, PlaceMode::TimingDriven) && options.delay.is_none() {
        bail!("timing-driven placement requires an explicit delay model");
    }

    design.stage = "placed".to_string();
    design.metadata.arch_name = options.arch.name.clone();
    apply_constraints(&mut design, &options.arch, &options.constraints);
    ensure_port_positions(&mut design, &options.arch);

    if matches!(options.mode, PlaceMode::TimingDriven) {
        annotate_net_criticality(&mut design);
    }

    if design.clusters.is_empty() {
        let mut report = StageReport::new("place");
        report.push("Design contains no clusters; placement only updated IO anchors.".to_string());
        return Ok(StageOutput {
            value: design,
            report,
        });
    }

    let block_ram_cluster_count = assign_block_ram_clusters(&mut design, &options.arch)?;
    let sites = options.arch.logic_sites();
    let site_capacity = options.arch.slices_per_tile.max(1);
    let logic_cluster_count = design
        .clusters
        .iter()
        .filter(|cluster| cluster.kind != ClusterKind::BlockRam)
        .count();
    if logic_cluster_count > sites.len().saturating_mul(site_capacity) {
        bail!(
            "not enough logic sites: need {}, only {} available",
            logic_cluster_count,
            sites.len().saturating_mul(site_capacity)
        );
    }

    let solution = solver::solve(&design, options)?;
    for (cluster, point) in design.clusters.iter_mut().zip(&solution.placements) {
        if let Some(point) = point {
            cluster.x = Some(point.x);
            cluster.y = Some(point.y);
        }
    }
    assign_cluster_slots(&mut design, site_capacity)?;

    let mut report = StageReport::new("place");
    report.metric("cluster_count", design.clusters.len());
    report.metric("grid_width", options.arch.width);
    report.metric("grid_height", options.arch.height);
    report.metric("site_capacity", site_capacity);
    report.metric("block_ram_cluster_count", block_ram_cluster_count);
    report.metric("mode", format!("{:?}", options.mode));
    report.metric("final_cost", solution.metrics.total);
    report.metric("wire_cost", solution.metrics.wire_cost);
    report.metric("congestion_cost", solution.metrics.congestion_cost);
    report.metric("timing_cost", solution.metrics.timing_cost);
    report.metric("locality_cost", solution.metrics.locality_cost);
    report.push(format!(
        "Placed {} clusters on a {}x{} grid with final cost {:.3}.",
        design.clusters.len(),
        options.arch.width,
        options.arch.height,
        solution.metrics.total
    ));
    report.push(format!(
        "Placement components: wire {:.3}, congestion {:.3}, timing {:.3}, locality {:.3}.",
        solution.metrics.wire_cost,
        solution.metrics.congestion_cost,
        solution.metrics.timing_cost,
        solution.metrics.locality_cost
    ));

    Ok(StageOutput {
        value: design,
        report,
    })
}

fn assign_block_ram_clusters(design: &mut Design, arch: &Arch) -> Result<usize> {
    let block_ram_sites = arch.block_ram_sites();
    let available = block_ram_sites.iter().copied().collect::<BTreeSet<_>>();
    let mut used = BTreeSet::<(usize, usize)>::new();
    let mut block_ram_clusters = design
        .clusters
        .iter_mut()
        .filter(|cluster| cluster.kind == ClusterKind::BlockRam)
        .collect::<Vec<_>>();
    if block_ram_clusters.is_empty() {
        return Ok(0);
    }
    if available.is_empty() {
        bail!("design contains block RAM clusters, but architecture exposes no block RAM sites");
    }

    for cluster in &mut block_ram_clusters {
        if let Some((x, y)) = cluster.x.zip(cluster.y) {
            if !available.contains(&(x, y)) {
                bail!(
                    "block RAM cluster {} is assigned to non-BRAM site ({x}, {y})",
                    cluster.name
                );
            }
            if !used.insert((x, y)) {
                bail!("multiple block RAM clusters request site ({x}, {y})");
            }
            cluster.z = Some(0);
            cluster.fixed = true;
        }
    }

    let mut remaining_sites = block_ram_sites
        .into_iter()
        .filter(|site| !used.contains(site))
        .collect::<Vec<_>>();
    remaining_sites.sort_unstable();
    let mut next_site = remaining_sites.into_iter();
    for cluster in &mut block_ram_clusters {
        if cluster.x.is_some() && cluster.y.is_some() {
            continue;
        }
        let Some((x, y)) = next_site.next() else {
            bail!(
                "not enough block RAM sites: need {}, only {} available",
                block_ram_clusters.len(),
                available.len()
            );
        };
        cluster.x = Some(x);
        cluster.y = Some(y);
        cluster.z = Some(0);
        cluster.fixed = true;
    }

    Ok(block_ram_clusters.len())
}

fn assign_cluster_slots(design: &mut Design, site_capacity: usize) -> Result<()> {
    let mut by_coordinate = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (index, cluster) in design.clusters.iter().enumerate() {
        let Some((x, y)) = cluster.x.zip(cluster.y) else {
            continue;
        };
        by_coordinate.entry((x, y)).or_default().push(index);
    }

    for ((x, y), cluster_indices) in by_coordinate {
        if cluster_indices.len() > site_capacity {
            bail!(
                "logic site ({x}, {y}) over capacity: {} clusters for {} slice slot(s)",
                cluster_indices.len(),
                site_capacity
            );
        }

        let mut remaining = cluster_indices;
        remaining.sort_by(|lhs, rhs| design.clusters[*lhs].name.cmp(&design.clusters[*rhs].name));
        let mut used = vec![false; site_capacity];

        for &cluster_index in &remaining {
            let requested = design.clusters[cluster_index].z;
            if let Some(slot) = requested {
                if slot >= site_capacity {
                    bail!(
                        "cluster {} requests slot {} at ({x}, {y}) but capacity is {}",
                        design.clusters[cluster_index].name,
                        slot,
                        site_capacity
                    );
                }
                if used[slot] {
                    bail!("multiple clusters request slot {} at ({x}, {y})", slot);
                }
                used[slot] = true;
            }
        }

        for cluster_index in remaining {
            let slot = if let Some(requested) = design.clusters[cluster_index].z {
                requested
            } else {
                used.iter()
                    .position(|occupied| !*occupied)
                    .ok_or_else(|| anyhow!("ran out of slice slots while assigning ({x}, {y})"))?
            };
            used[slot] = true;
            design.clusters[cluster_index].z = Some(slot);
        }
    }

    Ok(())
}

pub(crate) fn manhattan<L, R>(lhs: L, rhs: R) -> usize
where
    L: Into<Point>,
    R: Into<Point>,
{
    let lhs = lhs.into();
    let rhs = rhs.into();
    lhs.x.abs_diff(rhs.x) + lhs.y.abs_diff(rhs.y)
}
