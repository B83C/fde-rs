use crate::{ir::ClusterId, place::manhattan};
use anyhow::{Result, anyhow, bail};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use smallvec::SmallVec;

use super::{
    graph::ClusterGraph,
    model::{PlacementModel, Point},
};

pub(super) type ClusterUpdates = SmallVec<[(ClusterId, Point); 2]>;
pub(super) type PlacementBackups = SmallVec<[(ClusterId, Option<Point>); 2]>;
pub(super) type CandidateTargets = SmallVec<[Point; 16]>;
pub(super) type RankedSites = SmallVec<[(Point, usize); 8]>;
pub(super) type RankedNeighbors = SmallVec<[(ClusterId, f64); 3]>;
pub(super) type SiteOccupancy = SmallVec<[ClusterId; 2]>;
pub(super) type OccupancyMap = Vec<SiteOccupancy>;

pub(super) fn choose_focus(
    focus_weights: &[(ClusterId, f64)],
    rng: &mut ChaCha8Rng,
) -> Option<ClusterId> {
    let total = focus_weights.iter().map(|(_, weight)| *weight).sum::<f64>();
    if total <= 0.0 {
        return focus_weights.first().map(|(cluster_id, _)| *cluster_id);
    }
    let mut needle = rng.random::<f64>() * total;
    for (cluster_id, weight) in focus_weights {
        needle -= *weight;
        if needle <= 0.0 {
            return Some(*cluster_id);
        }
    }
    focus_weights.last().map(|(cluster_id, _)| *cluster_id)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn initial_placement(
    design: &crate::ir::Design,
    graph: &ClusterGraph,
    model: &PlacementModel,
    criticality: &[f64],
    sites: &[Point],
    site_mask: &[bool],
    width: usize,
    height: usize,
    site_capacity: usize,
) -> Result<Vec<Option<Point>>> {
    let mut placements = model.fixed_placements();
    let mut occupied = vec![SiteOccupancy::new(); width.saturating_mul(height).max(1)];

    for (index, cluster) in design.clusters.iter().enumerate() {
        if !cluster.fixed {
            continue;
        }
        let x = cluster
            .x
            .ok_or_else(|| anyhow!("fixed cluster {} is missing x", cluster.name))?;
        let y = cluster
            .y
            .ok_or_else(|| anyhow!("fixed cluster {} is missing y", cluster.name))?;
        let point = Point::new(x, y);
        if cluster.kind != crate::domain::ClusterKind::BlockRam
            && !site_contains(site_mask, point, width, height)
        {
            bail!(
                "fixed cluster {} is assigned to non-logic site ({}, {})",
                cluster.name,
                x,
                y
            );
        }
        if cluster.kind == crate::domain::ClusterKind::BlockRam {
            placements[index] = Some(point);
            continue;
        }
        let site_index = grid_index(point, width);
        if occupied
            .get(site_index)
            .is_some_and(|clusters| clusters.len() >= site_capacity)
        {
            bail!(
                "too many fixed clusters requested logic site ({}, {})",
                x,
                y
            );
        }
        occupied[site_index].push(ClusterId::new(index));
        placements[index] = Some(point);
    }

    let mut cluster_order = design
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, cluster)| !cluster.fixed)
        .map(|(index, _)| {
            let cluster_id = ClusterId::new(index);
            let graph_weight = graph.total_weight(cluster_id);
            let crit_weight = criticality.get(index).copied().unwrap_or(0.0);
            (cluster_id, graph_weight + crit_weight)
        })
        .collect::<Vec<_>>();
    cluster_order.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1).then_with(|| lhs.0.cmp(&rhs.0)));

    for (cluster_id, _) in cluster_order {
        let target = graph
            .weighted_centroid(cluster_id, &placements)
            .or_else(|| model.signal_centroid(cluster_id, &placements))
            .unwrap_or_else(|| sites[sites.len() / 2]);
        let site = nearest_available_site(target, sites, &occupied, width, site_capacity)
            .ok_or_else(|| anyhow!("ran out of logic sites during initial placement"))?;
        occupied[grid_index(site, width)].push(cluster_id);
        placements[cluster_id.index()] = Some(site);
    }

    Ok(placements)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn candidate_targets(
    focus: ClusterId,
    model: &PlacementModel,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    sites: &[Point],
    site_mask: &[bool],
    width: usize,
    height: usize,
    rng: &mut ChaCha8Rng,
) -> CandidateTargets {
    let mut targets = CandidateTargets::new();
    if let Some(current) = placements.get(focus.index()).copied().flatten() {
        push_unique(&mut targets, current);
        extend_best_sites(current, sites, 3, &mut targets);
    }

    if let Some(centroid) = graph.weighted_centroid(focus, placements) {
        extend_best_sites(centroid, sites, 5, &mut targets);
    }
    if let Some(signal_center) = model.signal_centroid(focus, placements) {
        extend_best_sites(signal_center, sites, 4, &mut targets);
    }

    for (neighbor, _) in best_neighbors(graph.neighbors(focus), 3) {
        if let Some(point) = placements.get(neighbor.index()).copied().flatten() {
            push_unique(&mut targets, point);
            for (nearby, _) in nearby_sites(point, site_mask, width, height, 1) {
                push_unique(&mut targets, nearby);
            }
        }
    }

    for _ in 0..3 {
        let site = sites[rng.random_range(0..sites.len())];
        push_unique(&mut targets, site);
    }

    targets
}

pub(super) fn extend_best_sites(
    target: Point,
    sites: &[Point],
    limit: usize,
    out: &mut CandidateTargets,
) {
    if limit == 0 {
        return;
    }

    let mut ranked = RankedSites::new();
    for site in sites {
        let distance = manhattan(*site, target);
        insert_ranked_site(&mut ranked, *site, distance, limit);
    }

    for (site, _) in ranked {
        push_unique(out, site);
    }
}

pub(super) fn nearby_sites(
    center: Point,
    site_mask: &[bool],
    width: usize,
    height: usize,
    radius: usize,
) -> RankedSites {
    let min_x = center.x.saturating_sub(radius);
    let min_y = center.y.saturating_sub(radius);
    let max_x = center.x.saturating_add(radius).min(width.saturating_sub(1));
    let max_y = center
        .y
        .saturating_add(radius)
        .min(height.saturating_sub(1));
    let mut result = RankedSites::new();
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            let point = Point::new(x, y);
            if site_contains(site_mask, point, width, height) {
                insert_ranked_site(&mut result, point, manhattan(point, center), usize::MAX);
            }
        }
    }
    result
}

pub(super) fn best_neighbors(neighbors: &[(ClusterId, f64)], limit: usize) -> RankedNeighbors {
    let mut ranked = RankedNeighbors::new();
    for &(cluster_id, weight) in neighbors {
        let insert_at = ranked
            .iter()
            .position(|(candidate, candidate_weight)| {
                (*candidate_weight, std::cmp::Reverse(*candidate))
                    < (weight, std::cmp::Reverse(cluster_id))
            })
            .unwrap_or(ranked.len());
        if insert_at < limit {
            ranked.insert(insert_at, (cluster_id, weight));
            if ranked.len() > limit {
                ranked.pop();
            }
        } else if ranked.len() < limit {
            ranked.push((cluster_id, weight));
        }
    }
    ranked
}

pub(super) fn insert_ranked_site(
    ranked: &mut RankedSites,
    site: Point,
    distance: usize,
    limit: usize,
) {
    let insert_at = ranked
        .iter()
        .position(|(candidate, candidate_distance)| {
            (*candidate_distance, *candidate) > (distance, site)
        })
        .unwrap_or(ranked.len());
    if insert_at < limit {
        ranked.insert(insert_at, (site, distance));
        if ranked.len() > limit {
            ranked.pop();
        }
    } else if ranked.len() < limit {
        ranked.push((site, distance));
    }
}

pub(super) fn plan_target_updates(
    placements: &[Option<Point>],
    occupancy: &[SiteOccupancy],
    movable_mask: &[bool],
    focus: ClusterId,
    target: Point,
    width: usize,
    site_capacity: usize,
) -> Option<ClusterUpdates> {
    let current = placements.get(focus.index()).copied().flatten()?;
    if current == target {
        return Some(SmallVec::new());
    }

    let occupants = occupancy.get(grid_index(target, width))?;

    let mut updates = SmallVec::<[(ClusterId, Point); 2]>::new();
    if occupants.len() < site_capacity {
        updates.push((focus, target));
    } else {
        let occupant = occupants.iter().copied().find(|cluster_id| {
            *cluster_id != focus
                && movable_mask
                    .get(cluster_id.index())
                    .copied()
                    .unwrap_or(false)
        })?;
        updates.push((focus, target));
        updates.push((occupant, current));
    }
    Some(updates)
}

pub(super) fn occupancy_map(
    placements: &[Option<Point>],
    width: usize,
    height: usize,
) -> OccupancyMap {
    let mut occupancy = vec![SiteOccupancy::new(); width.saturating_mul(height).max(1)];
    for (index, point) in placements.iter().enumerate() {
        let Some(point) = point else {
            continue;
        };
        let cell_index = grid_index(*point, width);
        if let Some(slot) = occupancy.get_mut(cell_index) {
            slot.push(ClusterId::new(index));
        }
    }
    occupancy
}

pub(super) fn apply_updates_in_place(
    placements: &mut [Option<Point>],
    updates: &[(ClusterId, Point)],
) -> PlacementBackups {
    let mut backups = PlacementBackups::new();
    for (cluster_id, position) in updates {
        if let Some(slot) = placements.get_mut(cluster_id.index()) {
            backups.push((*cluster_id, *slot));
            *slot = Some(*position);
        }
    }
    backups
}

pub(super) fn restore_updates(
    placements: &mut [Option<Point>],
    backups: &[(ClusterId, Option<Point>)],
) {
    for (cluster_id, position) in backups.iter().rev() {
        if let Some(slot) = placements.get_mut(cluster_id.index()) {
            *slot = *position;
        }
    }
}

pub(super) fn random_swap_updates(
    placements: &[Option<Point>],
    movable: &[ClusterId],
    rng: &mut ChaCha8Rng,
) -> Option<ClusterUpdates> {
    if movable.len() < 2 {
        return None;
    }
    let lhs_index = rng.random_range(0..movable.len());
    let mut rhs_index = rng.random_range(0..movable.len());
    while rhs_index == lhs_index {
        rhs_index = rng.random_range(0..movable.len());
    }
    let lhs = movable[lhs_index];
    let rhs = movable[rhs_index];
    let lhs_pos = placements.get(lhs.index()).copied().flatten()?;
    let rhs_pos = placements.get(rhs.index()).copied().flatten()?;
    let mut updates = SmallVec::<[(ClusterId, Point); 2]>::new();
    updates.push((lhs, rhs_pos));
    updates.push((rhs, lhs_pos));
    Some(updates)
}

pub(super) fn nearest_available_site(
    target: Point,
    sites: &[Point],
    occupied: &[SiteOccupancy],
    width: usize,
    site_capacity: usize,
) -> Option<Point> {
    sites
        .iter()
        .filter(|site| {
            occupied
                .get(grid_index(**site, width))
                .is_some_and(|clusters| clusters.len() < site_capacity)
        })
        .min_by(|lhs, rhs| {
            manhattan(**lhs, target)
                .cmp(&manhattan(**rhs, target))
                .then_with(|| lhs.cmp(rhs))
        })
        .copied()
}

pub(super) fn site_mask(sites: &[Point], width: usize, height: usize) -> Vec<bool> {
    let mut mask = vec![false; width.saturating_mul(height).max(1)];
    for site in sites {
        let index = grid_index(*site, width);
        if let Some(slot) = mask.get_mut(index) {
            *slot = true;
        }
    }
    mask
}

pub(super) fn site_contains(site_mask: &[bool], point: Point, width: usize, height: usize) -> bool {
    if point.x >= width || point.y >= height {
        return false;
    }
    site_mask
        .get(grid_index(point, width))
        .copied()
        .unwrap_or(false)
}

pub(super) fn push_unique(points: &mut CandidateTargets, point: Point) {
    if !points.contains(&point) {
        points.push(point);
    }
}

pub(super) fn grid_index(point: Point, width: usize) -> usize {
    point.y.saturating_mul(width).saturating_add(point.x)
}
