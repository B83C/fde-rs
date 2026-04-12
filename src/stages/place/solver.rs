use crate::{
    ir::ClusterId,
    place::{PlaceMode, PlaceOptions},
    report::{StageReporter, emit_stage_info, emit_stage_progress},
};
use anyhow::{Result, anyhow};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::{
    cost::{PlacementCandidate, PlacementEvaluator, PlacementMetrics, evaluate_positions},
    graph::{ClusterGraph, build_cluster_graph, cluster_incident_criticality},
    model::{PlacementModel, Point},
    support::{
        CandidateTargets, ClusterUpdates, OccupancyMap, SiteOccupancy, apply_updates_in_place,
        best_neighbors, candidate_targets, choose_focus, extend_best_sites, initial_placement,
        nearby_sites, occupancy_map, plan_target_updates, push_unique, random_swap_updates,
        restore_updates, site_mask,
    },
};

const INCREMENTAL_EVALUATOR_NET_THRESHOLD: usize = 128;

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
    let mut logger = None;
    solve_internal(design, options, &mut logger, None)
}

pub(crate) fn solve_with_reporter(
    design: &crate::ir::Design,
    options: &PlaceOptions,
    reporter: &mut dyn StageReporter,
) -> Result<PlacementSolution> {
    solve_internal(design, options, &mut Some(reporter), None)
}

fn solve_internal(
    design: &crate::ir::Design,
    options: &PlaceOptions,
    reporter: &mut Option<&mut dyn StageReporter>,
    incremental_override: Option<bool>,
) -> Result<PlacementSolution> {
    if reporter.as_deref().is_some_and(StageReporter::is_cancelled) {
        return Err(anyhow!("placement cancelled"));
    }

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
    emit_stage_info(
        reporter,
        "place",
        format!(
            "placement solver initialized: movable_clusters={}, nets={}, strategy={}",
            movable.len(),
            model.nets.len(),
            if use_incremental {
                "incremental"
            } else {
                "full"
            }
        ),
    );
    if use_incremental {
        solve_incremental(&context, reporter)
    } else {
        solve_full(&context, reporter)
    }
}

fn should_log_progress(step: usize, iterations: usize) -> bool {
    if iterations <= 20 {
        return true;
    }
    let interval = (iterations / 20).max(1);
    step == 0 || step + 1 == iterations || (step + 1).is_multiple_of(interval)
}

fn emit_anneal_progress(
    reporter: &mut Option<&mut dyn StageReporter>,
    strategy: &str,
    step: usize,
    iterations: usize,
    temperature: f64,
    current: &PlacementMetrics,
    best: &PlacementMetrics,
) {
    emit_stage_progress(
        reporter,
        "place",
        format!(
            "{} anneal {}/{} ({:.0}%), temp={:.3}, current={:.3}, best={:.3}",
            strategy,
            step + 1,
            iterations,
            ((step + 1) as f64 / iterations.max(1) as f64) * 100.0,
            temperature,
            current.total,
            best.total
        ),
    );
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

fn solve_incremental(
    context: &SolveContext<'_>,
    reporter: &mut Option<&mut dyn StageReporter>,
) -> Result<PlacementSolution> {
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
    emit_stage_info(
        reporter,
        "place",
        format!(
            "starting incremental anneal with {} iterations, initial cost {:.3}",
            iterations, state.metrics.total
        ),
    );

    for step in 0..iterations {
        if reporter.as_deref().is_some_and(StageReporter::is_cancelled) {
            return Err(anyhow!("placement cancelled"));
        }
        if should_log_progress(step, iterations) {
            emit_anneal_progress(
                reporter,
                "incremental",
                step,
                iterations,
                temperature,
                &state.metrics,
                &best.metrics,
            );
        }
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

    emit_stage_info(
        reporter,
        "place",
        format!(
            "incremental anneal finished with best cost {:.3}; starting refinement",
            best.metrics.total
        ),
    );
    refine_solution(context, best.placements, best.metrics, reporter)
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

fn solve_full(
    context: &SolveContext<'_>,
    reporter: &mut Option<&mut dyn StageReporter>,
) -> Result<PlacementSolution> {
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
    emit_stage_info(
        reporter,
        "place",
        format!(
            "starting full anneal with {} iterations, initial cost {:.3}",
            iterations, state.metrics.total
        ),
    );

    for step in 0..iterations {
        if reporter.as_deref().is_some_and(StageReporter::is_cancelled) {
            return Err(anyhow!("placement cancelled"));
        }
        if should_log_progress(step, iterations) {
            emit_anneal_progress(
                reporter,
                "full",
                step,
                iterations,
                temperature,
                &state.metrics,
                &best.metrics,
            );
        }
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

    emit_stage_info(
        reporter,
        "place",
        format!(
            "full anneal finished with best cost {:.3}; starting refinement",
            best.metrics.total
        ),
    );
    refine_solution(context, best.placements, best.metrics, reporter)
}

fn refine_solution(
    context: &SolveContext<'_>,
    placements: Vec<Option<Point>>,
    metrics: PlacementMetrics,
    reporter: &mut Option<&mut dyn StageReporter>,
) -> Result<PlacementSolution> {
    let mut evaluator = PlacementEvaluator::new_from_positions(
        context.model,
        context.graph,
        placements,
        &context.options.arch,
        context.options.delay.as_deref(),
        context.options.mode,
    );
    if evaluator.metrics().total > metrics.total + 1e-9 {
        return Ok(PlacementSolution {
            placements: evaluator.placements().to_vec(),
            metrics: evaluator.metrics().clone(),
        });
    }

    let mut occupancy = occupancy_map(
        evaluator.placements(),
        context.options.arch.width,
        context.options.arch.height,
    );
    let focus_order = refinement_focus_order(context);
    let pass_limit = refinement_pass_limit(context.movable.len());
    emit_stage_info(
        reporter,
        "place",
        format!(
            "refinement configured for up to {} pass(es) across {} focus clusters",
            pass_limit,
            focus_order.len()
        ),
    );

    for pass_index in 0..pass_limit {
        if reporter.as_deref().is_some_and(StageReporter::is_cancelled) {
            return Err(anyhow!("placement cancelled"));
        }
        let mut improved = false;
        for &focus in &focus_order {
            if reporter.as_deref().is_some_and(StageReporter::is_cancelled) {
                return Err(anyhow!("placement cancelled"));
            }
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
        emit_stage_progress(
            reporter,
            "place",
            format!(
                "refinement pass {}/{} -> {} (cost {:.3})",
                pass_index + 1,
                pass_limit,
                if improved { "improved" } else { "stable" },
                evaluator.metrics().total
            ),
        );
        if !improved {
            break;
        }
    }

    emit_stage_info(
        reporter,
        "place",
        format!(
            "placement refinement complete with final cost {:.3}",
            evaluator.metrics().total
        ),
    );
    Ok(PlacementSolution {
        placements: evaluator.placements().to_vec(),
        metrics: evaluator.metrics().clone(),
    })
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
