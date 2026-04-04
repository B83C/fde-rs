use crate::{
    ir::ClusterId,
    place::{PlaceMode, PlaceOptions, manhattan},
};
use anyhow::{Result, anyhow, bail};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use smallvec::SmallVec;

use super::{
    cost::{PlacementCandidate, PlacementEvaluator, PlacementMetrics, evaluate_positions},
    graph::{ClusterGraph, build_cluster_graph, cluster_incident_criticality},
    model::{PlacementModel, Point},
};

const INCREMENTAL_EVALUATOR_NET_THRESHOLD: usize = 128;

type ClusterUpdates = SmallVec<[(ClusterId, Point); 2]>;
type PlacementBackups = SmallVec<[(ClusterId, Option<Point>); 2]>;
type CandidateTargets = SmallVec<[Point; 16]>;
type RankedSites = SmallVec<[(Point, usize); 8]>;
type RankedNeighbors = SmallVec<[(ClusterId, f64); 3]>;
type SiteOccupancy = SmallVec<[ClusterId; 2]>;
type OccupancyMap = Vec<SiteOccupancy>;

#[derive(Debug, Clone)]
pub(crate) struct PlacementSolution {
    pub(crate) placements: Vec<Option<Point>>,
    pub(crate) metrics: PlacementMetrics,
}

struct SolveContext<'a> {
    design: &'a crate::ir::Design,
    options: &'a PlaceOptions,
    graph: &'a ClusterGraph,
    model: &'a PlacementModel,
    criticality: &'a [f64],
    sites: &'a [Point],
    site_mask: &'a [bool],
    movable: &'a [ClusterId],
    movable_mask: &'a [bool],
}

struct IncrementalAnnealState<'a> {
    evaluator: PlacementEvaluator<'a>,
    occupancy: OccupancyMap,
    metrics: PlacementMetrics,
}

struct FullAnnealState {
    current: Vec<Option<Point>>,
    trial: Vec<Option<Point>>,
    occupancy: OccupancyMap,
    metrics: PlacementMetrics,
}

pub(crate) fn solve(
    design: &crate::ir::Design,
    options: &PlaceOptions,
) -> Result<PlacementSolution> {
    solve_internal(design, options, None)
}

fn solve_internal(
    design: &crate::ir::Design,
    options: &PlaceOptions,
    incremental_override: Option<bool>,
) -> Result<PlacementSolution> {
    let sites = options
        .arch
        .logic_sites()
        .into_iter()
        .map(Point::from)
        .collect::<Vec<_>>();
    let site_mask = site_mask(&sites, options.arch.width, options.arch.height);
    let graph = build_cluster_graph(design);
    let model = PlacementModel::from_design(design);
    let criticality = cluster_incident_criticality(design);
    let movable = design
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, cluster)| !cluster.fixed)
        .map(|(index, _)| ClusterId::new(index))
        .collect::<Vec<_>>();
    let mut movable_mask = vec![false; design.clusters.len()];
    for cluster_id in &movable {
        movable_mask[cluster_id.index()] = true;
    }

    if movable.len() <= 1 {
        let current = initial_placement(
            design,
            &graph,
            &model,
            &criticality,
            &sites,
            &site_mask,
            options.arch.width,
            options.arch.height,
            options.arch.slices_per_tile.max(1),
        )?;
        let metrics = evaluate_positions(
            &model,
            &graph,
            &current,
            &options.arch,
            options.delay.as_deref(),
            options.mode,
        );
        return Ok(PlacementSolution {
            placements: current,
            metrics,
        });
    }

    let context = SolveContext {
        design,
        options,
        graph: &graph,
        model: &model,
        criticality: &criticality,
        sites: &sites,
        site_mask: &site_mask,
        movable: &movable,
        movable_mask: &movable_mask,
    };

    let use_incremental =
        incremental_override.unwrap_or(model.nets.len() >= INCREMENTAL_EVALUATOR_NET_THRESHOLD);
    if use_incremental {
        solve_incremental(&context)
    } else {
        solve_full(&context)
    }
}

fn initial_positions(context: &SolveContext<'_>) -> Result<Vec<Option<Point>>> {
    initial_placement(
        context.design,
        context.graph,
        context.model,
        context.criticality,
        context.sites,
        context.site_mask,
        context.options.arch.width,
        context.options.arch.height,
        context.options.arch.slices_per_tile.max(1),
    )
}

fn anneal_iterations(context: &SolveContext<'_>) -> usize {
    700 + context.movable.len() * 50
}

fn anneal_temperature(total_cost: f64, movable_count: usize) -> f64 {
    (total_cost / movable_count.max(1) as f64).max(0.5)
}

fn cool_temperature(temperature: f64, step: usize) -> f64 {
    let cooled = temperature
        * if step.is_multiple_of(40) {
            0.985
        } else {
            0.9985
        };
    cooled.max(0.02)
}

fn stall_limit(context: &SolveContext<'_>) -> usize {
    context.movable.len() * 3
}

fn choose_focus_and_targets(
    context: &SolveContext<'_>,
    placements: &[Option<Point>],
    focus_weights: &[(ClusterId, f64)],
    rng: &mut ChaCha8Rng,
) -> Result<(ClusterId, CandidateTargets)> {
    let focus = choose_focus(focus_weights, rng)
        .ok_or_else(|| anyhow!("missing movable cluster during placement"))?;
    let targets = candidate_targets(
        focus,
        context.model,
        context.graph,
        placements,
        context.sites,
        context.site_mask,
        context.options.arch.width,
        context.options.arch.height,
        rng,
    );
    Ok((focus, targets))
}

fn accept_trial(
    rng: &mut ChaCha8Rng,
    current_total: f64,
    trial_total: f64,
    temperature: f64,
) -> bool {
    if trial_total + 1e-9 < current_total {
        return true;
    }
    let delta = trial_total - current_total;
    let threshold = (-delta / temperature.max(0.01)).exp().clamp(0.0, 1.0);
    rng.random::<f64>() < threshold
}

fn update_best_solution(
    best: &mut PlacementSolution,
    current: &[Option<Point>],
    current_metrics: &PlacementMetrics,
) -> bool {
    if current_metrics.total + 1e-9 >= best.metrics.total {
        return false;
    }
    best.placements.as_mut_slice().clone_from_slice(current);
    best.metrics = current_metrics.clone();
    true
}

fn incremental_state<'a>(
    context: &'a SolveContext<'a>,
    placements: Vec<Option<Point>>,
) -> IncrementalAnnealState<'a> {
    let evaluator = PlacementEvaluator::new_from_positions(
        context.model,
        context.graph,
        placements,
        &context.options.arch,
        context.options.delay.as_deref(),
        context.options.mode,
    );
    let occupancy = occupancy_map(
        evaluator.placements(),
        context.options.arch.width,
        context.options.arch.height,
    );
    let metrics = evaluator.metrics().clone();
    IncrementalAnnealState {
        evaluator,
        occupancy,
        metrics,
    }
}

fn full_state(context: &SolveContext<'_>, current: Vec<Option<Point>>) -> FullAnnealState {
    let occupancy = occupancy_map(
        &current,
        context.options.arch.width,
        context.options.arch.height,
    );
    let metrics = evaluate_positions(
        context.model,
        context.graph,
        &current,
        &context.options.arch,
        context.options.delay.as_deref(),
        context.options.mode,
    );
    let trial = current.clone();
    FullAnnealState {
        current,
        trial,
        occupancy,
        metrics,
    }
}

fn best_incremental_trial(
    context: &SolveContext<'_>,
    evaluator: &mut PlacementEvaluator<'_>,
    current_occupancy: &[SiteOccupancy],
    focus: ClusterId,
    candidates: CandidateTargets,
) -> Option<PlacementCandidate> {
    let mut best_updates: Option<ClusterUpdates> = None;
    let mut best_total = f64::INFINITY;
    for target in candidates {
        let Some(changes) = plan_target_updates(
            evaluator.placements(),
            current_occupancy,
            context.movable_mask,
            focus,
            target,
            context.options.arch.width,
            context.options.arch.slices_per_tile.max(1),
        ) else {
            continue;
        };
        let metrics = evaluator.evaluate_candidate_metrics(&changes);
        if metrics.total < best_total {
            best_total = metrics.total;
            best_updates = Some(changes);
        }
    }

    best_updates.map(|changes| evaluator.evaluate_candidate(&changes))
}

fn maybe_apply_incremental_swap(
    context: &SolveContext<'_>,
    state: &mut IncrementalAnnealState<'_>,
    best: &mut PlacementSolution,
    rng: &mut ChaCha8Rng,
) {
    if let Some(swapped) = random_swap_updates(state.evaluator.placements(), context.movable, rng) {
        let swap_metrics = state.evaluator.evaluate_candidate_metrics(&swapped);
        if swap_metrics.total < state.metrics.total {
            let swap_candidate = state.evaluator.evaluate_candidate(&swapped);
            state.evaluator.apply_candidate(swap_candidate);
            state.occupancy = occupancy_map(
                state.evaluator.placements(),
                context.options.arch.width,
                context.options.arch.height,
            );
            state.metrics = swap_metrics;
            update_best_solution(best, state.evaluator.placements(), &state.metrics);
        }
    }
}

fn solve_incremental(context: &SolveContext<'_>) -> Result<PlacementSolution> {
    let mut rng = ChaCha8Rng::seed_from_u64(context.options.seed);
    let mut state = incremental_state(context, initial_positions(context)?);
    let mut best = PlacementSolution {
        placements: state.evaluator.placements().to_vec(),
        metrics: state.evaluator.metrics().clone(),
    };
    let focus_weights = focus_weights(context);

    let iterations = anneal_iterations(context);
    let mut temperature = anneal_temperature(state.metrics.total, context.movable.len());
    let mut stall = 0usize;

    for step in 0..iterations {
        let (focus, candidates) = choose_focus_and_targets(
            context,
            state.evaluator.placements(),
            &focus_weights,
            &mut rng,
        )?;
        let best_trial = best_incremental_trial(
            context,
            &mut state.evaluator,
            &state.occupancy,
            focus,
            candidates,
        );

        let Some(trial) = best_trial else {
            continue;
        };
        let trial_metrics = trial.metrics().clone();
        let accept = accept_trial(
            &mut rng,
            state.metrics.total,
            trial_metrics.total,
            temperature,
        );

        if accept {
            state.evaluator.apply_candidate(trial);
            state.occupancy = occupancy_map(
                state.evaluator.placements(),
                context.options.arch.width,
                context.options.arch.height,
            );
            state.metrics = trial_metrics;
            if update_best_solution(&mut best, state.evaluator.placements(), &state.metrics) {
                stall = 0;
            } else {
                stall += 1;
            }
        } else {
            stall += 1;
        }

        if stall > stall_limit(context) {
            maybe_apply_incremental_swap(context, &mut state, &mut best, &mut rng);
            stall = 0;
        }

        temperature = cool_temperature(temperature, step);
    }

    Ok(refine_solution(context, best.placements, best.metrics))
}

fn best_full_trial(
    context: &SolveContext<'_>,
    current: &[Option<Point>],
    trial: &mut [Option<Point>],
    current_occupancy: &[SiteOccupancy],
    focus: ClusterId,
    candidates: CandidateTargets,
) -> Option<(ClusterUpdates, PlacementMetrics)> {
    let mut best_trial: Option<(ClusterUpdates, PlacementMetrics)> = None;
    for target in candidates {
        let Some(changes) = plan_target_updates(
            current,
            current_occupancy,
            context.movable_mask,
            focus,
            target,
            context.options.arch.width,
            context.options.arch.slices_per_tile.max(1),
        ) else {
            continue;
        };
        let backups = apply_updates_in_place(trial, &changes);
        let metrics = evaluate_positions(
            context.model,
            context.graph,
            trial,
            &context.options.arch,
            context.options.delay.as_deref(),
            context.options.mode,
        );
        restore_updates(trial, &backups);
        if best_trial
            .as_ref()
            .is_none_or(|(_, best_metrics)| metrics.total < best_metrics.total)
        {
            best_trial = Some((changes, metrics));
        }
    }
    best_trial
}

fn maybe_apply_full_swap(
    context: &SolveContext<'_>,
    state: &mut FullAnnealState,
    best: &mut PlacementSolution,
    rng: &mut ChaCha8Rng,
) {
    if let Some(swapped) = random_swap_updates(&state.current, context.movable, rng) {
        let backups = apply_updates_in_place(&mut state.trial, &swapped);
        let swap_metrics = evaluate_positions(
            context.model,
            context.graph,
            &state.trial,
            &context.options.arch,
            context.options.delay.as_deref(),
            context.options.mode,
        );
        restore_updates(&mut state.trial, &backups);
        if swap_metrics.total < state.metrics.total {
            apply_updates_in_place(&mut state.current, &swapped);
            apply_updates_in_place(&mut state.trial, &swapped);
            state.occupancy = occupancy_map(
                &state.current,
                context.options.arch.width,
                context.options.arch.height,
            );
            state.metrics = swap_metrics;
            update_best_solution(best, &state.current, &state.metrics);
        }
    }
}

fn solve_full(context: &SolveContext<'_>) -> Result<PlacementSolution> {
    let mut rng = ChaCha8Rng::seed_from_u64(context.options.seed);
    let mut state = full_state(context, initial_positions(context)?);
    let mut best = PlacementSolution {
        placements: state.current.clone(),
        metrics: state.metrics.clone(),
    };
    let focus_weights = focus_weights(context);

    let iterations = anneal_iterations(context);
    let mut temperature = anneal_temperature(state.metrics.total, context.movable.len());
    let mut stall = 0usize;

    for step in 0..iterations {
        let (focus, candidates) =
            choose_focus_and_targets(context, &state.current, &focus_weights, &mut rng)?;
        let best_trial = best_full_trial(
            context,
            &state.current,
            &mut state.trial,
            &state.occupancy,
            focus,
            candidates,
        );

        let Some((trial_updates, trial_metrics)) = best_trial else {
            continue;
        };
        let accept = accept_trial(
            &mut rng,
            state.metrics.total,
            trial_metrics.total,
            temperature,
        );

        if accept {
            apply_updates_in_place(&mut state.current, &trial_updates);
            apply_updates_in_place(&mut state.trial, &trial_updates);
            state.occupancy = occupancy_map(
                &state.current,
                context.options.arch.width,
                context.options.arch.height,
            );
            state.metrics = trial_metrics;
            if update_best_solution(&mut best, &state.current, &state.metrics) {
                stall = 0;
            } else {
                stall += 1;
            }
        } else {
            stall += 1;
        }

        if stall > stall_limit(context) {
            maybe_apply_full_swap(context, &mut state, &mut best, &mut rng);
            stall = 0;
        }

        temperature = cool_temperature(temperature, step);
    }

    Ok(refine_solution(context, best.placements, best.metrics))
}

fn refine_solution(
    context: &SolveContext<'_>,
    placements: Vec<Option<Point>>,
    metrics: PlacementMetrics,
) -> PlacementSolution {
    let mut evaluator = PlacementEvaluator::new_from_positions(
        context.model,
        context.graph,
        placements,
        &context.options.arch,
        context.options.delay.as_deref(),
        context.options.mode,
    );
    if evaluator.metrics().total > metrics.total + 1e-9 {
        return PlacementSolution {
            placements: evaluator.placements().to_vec(),
            metrics: evaluator.metrics().clone(),
        };
    }

    let mut occupancy = occupancy_map(
        evaluator.placements(),
        context.options.arch.width,
        context.options.arch.height,
    );
    let focus_order = refinement_focus_order(context);
    let pass_limit = refinement_pass_limit(context.movable.len());

    for _ in 0..pass_limit {
        let mut improved = false;
        for &focus in &focus_order {
            let candidates = refinement_targets(context, focus, evaluator.placements());
            let mut best_trial: Option<PlacementCandidate> = None;
            for target in candidates {
                let Some(changes) = plan_target_updates(
                    evaluator.placements(),
                    &occupancy,
                    context.movable_mask,
                    focus,
                    target,
                    context.options.arch.width,
                    context.options.arch.slices_per_tile.max(1),
                ) else {
                    continue;
                };
                if changes.is_empty() {
                    continue;
                }
                let trial_metrics = evaluator.evaluate_candidate_metrics(&changes);
                if trial_metrics.total + 1e-9 >= evaluator.metrics().total {
                    continue;
                }
                let trial = evaluator.evaluate_candidate(&changes);
                if best_trial
                    .as_ref()
                    .is_none_or(|best| trial.metrics().total + 1e-9 < best.metrics().total)
                {
                    best_trial = Some(trial);
                }
            }

            if let Some(trial) = best_trial {
                evaluator.apply_candidate(trial);
                occupancy = occupancy_map(
                    evaluator.placements(),
                    context.options.arch.width,
                    context.options.arch.height,
                );
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }

    PlacementSolution {
        placements: evaluator.placements().to_vec(),
        metrics: evaluator.metrics().clone(),
    }
}

fn refinement_focus_order(context: &SolveContext<'_>) -> Vec<ClusterId> {
    let mut order = focus_weights(context);
    order.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1).then_with(|| lhs.0.cmp(&rhs.0)));
    order
        .into_iter()
        .map(|(cluster_id, _)| cluster_id)
        .collect()
}

fn refinement_pass_limit(movable_count: usize) -> usize {
    if movable_count <= 16 {
        3
    } else if movable_count <= 96 {
        2
    } else {
        1
    }
}

fn refinement_targets(
    context: &SolveContext<'_>,
    focus: ClusterId,
    placements: &[Option<Point>],
) -> CandidateTargets {
    let mut targets = CandidateTargets::new();
    let Some(current) = placements.get(focus.index()).copied().flatten() else {
        return targets;
    };
    push_unique(&mut targets, current);
    for (nearby, _) in nearby_sites(
        current,
        context.site_mask,
        context.options.arch.width,
        context.options.arch.height,
        2,
    ) {
        push_unique(&mut targets, nearby);
    }

    if let Some(centroid) = context.graph.weighted_centroid(focus, placements) {
        extend_best_sites(centroid, context.sites, 4, &mut targets);
    }
    if let Some(signal_center) = context.model.signal_centroid(focus, placements) {
        extend_best_sites(signal_center, context.sites, 4, &mut targets);
    }
    for (neighbor, _) in best_neighbors(context.graph.neighbors(focus), 4) {
        if let Some(point) = placements.get(neighbor.index()).copied().flatten() {
            push_unique(&mut targets, point);
            for (nearby, _) in nearby_sites(
                point,
                context.site_mask,
                context.options.arch.width,
                context.options.arch.height,
                1,
            ) {
                push_unique(&mut targets, nearby);
            }
        }
    }
    targets
}

fn focus_weights(context: &SolveContext<'_>) -> Vec<(ClusterId, f64)> {
    context
        .movable
        .iter()
        .map(|cluster_id| {
            let graph_weight = context.graph.total_weight(*cluster_id);
            let crit_weight = context
                .criticality
                .get(cluster_id.index())
                .copied()
                .unwrap_or(0.0);
            let weight = match context.options.mode {
                PlaceMode::BoundingBox => 1.0 + graph_weight,
                PlaceMode::TimingDriven => 1.0 + graph_weight + 1.5 * crit_weight,
            };
            (*cluster_id, weight.max(0.1))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn initial_placement(
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
        if !site_contains(site_mask, point, width, height) {
            bail!(
                "fixed cluster {} is assigned to non-logic site ({}, {})",
                cluster.name,
                x,
                y
            );
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

fn choose_focus(focus_weights: &[(ClusterId, f64)], rng: &mut ChaCha8Rng) -> Option<ClusterId> {
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
fn candidate_targets(
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

fn extend_best_sites(target: Point, sites: &[Point], limit: usize, out: &mut CandidateTargets) {
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

fn nearby_sites(
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

fn best_neighbors(neighbors: &[(ClusterId, f64)], limit: usize) -> RankedNeighbors {
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

fn insert_ranked_site(ranked: &mut RankedSites, site: Point, distance: usize, limit: usize) {
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

fn plan_target_updates(
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

fn occupancy_map(placements: &[Option<Point>], width: usize, height: usize) -> OccupancyMap {
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

fn apply_updates_in_place(
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

fn restore_updates(placements: &mut [Option<Point>], backups: &[(ClusterId, Option<Point>)]) {
    for (cluster_id, position) in backups.iter().rev() {
        if let Some(slot) = placements.get_mut(cluster_id.index()) {
            *slot = *position;
        }
    }
}

fn random_swap_updates(
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

fn nearest_available_site(
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

fn site_mask(sites: &[Point], width: usize, height: usize) -> Vec<bool> {
    let mut mask = vec![false; width.saturating_mul(height).max(1)];
    for site in sites {
        let index = grid_index(*site, width);
        if let Some(slot) = mask.get_mut(index) {
            *slot = true;
        }
    }
    mask
}

fn site_contains(site_mask: &[bool], point: Point, width: usize, height: usize) -> bool {
    if point.x >= width || point.y >= height {
        return false;
    }
    site_mask
        .get(grid_index(point, width))
        .copied()
        .unwrap_or(false)
}

fn push_unique(points: &mut CandidateTargets, point: Point) {
    if !points.contains(&point) {
        points.push(point);
    }
}

fn grid_index(point: Point, width: usize) -> usize {
    point.y.saturating_mul(width).saturating_add(point.x)
}
