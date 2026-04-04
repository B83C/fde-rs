use crate::{
    ir::ClusterId,
    place::{PlaceMode, manhattan},
    resource::{Arch, DelayModel},
};
use rayon::prelude::*;
use smallvec::SmallVec;

use super::{
    graph::ClusterGraph,
    model::{PlacementModel, Point, PreparedNet},
};

const CONGESTION_THRESHOLD: f64 = 1.35;
const CONGESTION_SCALE: f64 = 2.5;
const PARALLEL_NET_THRESHOLD: usize = 256;

type ClusterUpdates = SmallVec<[(ClusterId, Point); 2]>;

#[derive(Debug, Clone, Default)]
pub(crate) struct PlacementMetrics {
    pub(crate) wire_cost: f64,
    pub(crate) congestion_cost: f64,
    pub(crate) timing_cost: f64,
    pub(crate) locality_cost: f64,
    pub(crate) total: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct PlacementEvaluator<'a> {
    model: &'a PlacementModel,
    graph: &'a ClusterGraph,
    placements: Vec<Option<Point>>,
    arch: &'a Arch,
    delay: Option<&'a DelayModel>,
    mode: PlaceMode,
    net_models: Vec<Option<NetModel>>,
    loads: Vec<f64>,
    locality_terms: Vec<f64>,
    locality_weights: Vec<f64>,
    congestion_score_raw: f64,
    metrics: PlacementMetrics,
    scratch: EvaluationScratch,
}

#[derive(Debug, Clone)]
pub(crate) struct PlacementCandidate {
    updates: ClusterUpdates,
    net_updates: Vec<(usize, Option<NetModel>)>,
    load_deltas: Vec<(usize, f64)>,
    locality_updates: Vec<(ClusterId, f64)>,
    metrics: PlacementMetrics,
}

#[derive(Debug, Clone)]
struct NetModel {
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
    area: usize,
    hpwl: f64,
    route_delay: f64,
    driver_span: f64,
    weight: f64,
}

struct EvaluationState {
    net_models: Vec<Option<NetModel>>,
    loads: Vec<f64>,
    locality_terms: Vec<f64>,
    locality_weights: Vec<f64>,
    congestion_score_raw: f64,
    metrics: PlacementMetrics,
}

#[derive(Debug, Clone)]
struct EvaluationScratch {
    affected_nets_seen: Vec<bool>,
    affected_nets: Vec<usize>,
    load_deltas: Vec<f64>,
    touched_loads: Vec<usize>,
    touched_mask: Vec<bool>,
    affected_locality_seen: Vec<bool>,
    affected_locality: Vec<ClusterId>,
}

struct CandidateNetEffects {
    wire_cost: f64,
    timing_cost: f64,
    congestion_score_raw: f64,
    load_deltas: Vec<(usize, f64)>,
    net_updates: Vec<(usize, Option<NetModel>)>,
}

struct CandidateLocalityEffects {
    cost: f64,
    updates: Vec<(ClusterId, f64)>,
}

struct CandidateNetMetrics {
    wire_cost: f64,
    timing_cost: f64,
    congestion_score_raw: f64,
}

impl<'a> PlacementEvaluator<'a> {
    pub(crate) fn new_from_positions(
        model: &'a PlacementModel,
        graph: &'a ClusterGraph,
        placements: Vec<Option<Point>>,
        arch: &'a Arch,
        delay: Option<&'a DelayModel>,
        mode: PlaceMode,
    ) -> Self {
        let evaluation = build_evaluation_state(model, graph, &placements, arch, delay, mode);

        Self {
            model,
            graph,
            placements,
            arch,
            delay,
            mode,
            net_models: evaluation.net_models,
            loads: evaluation.loads,
            locality_terms: evaluation.locality_terms,
            locality_weights: evaluation.locality_weights,
            congestion_score_raw: evaluation.congestion_score_raw,
            metrics: evaluation.metrics,
            scratch: EvaluationScratch::new(model, arch),
        }
    }

    pub(crate) fn placements(&self) -> &[Option<Point>] {
        &self.placements
    }

    pub(crate) fn metrics(&self) -> &PlacementMetrics {
        &self.metrics
    }

    pub(crate) fn evaluate_candidate<P>(&mut self, updates: &[(ClusterId, P)]) -> PlacementCandidate
    where
        P: Copy + Into<Point>,
    {
        let updates = normalize_candidate_updates(updates);
        if updates.is_empty() {
            return self.empty_candidate();
        }

        let moved_clusters = moved_clusters(&updates);
        let net_effects = self.candidate_net_effects(&updates, &moved_clusters);
        let locality_effects = self.candidate_locality_effects(&updates, &moved_clusters);

        let metrics = compose_metrics(
            self.mode,
            net_effects.wire_cost,
            net_effects.congestion_score_raw,
            net_effects.timing_cost,
            locality_effects.cost,
        );

        PlacementCandidate {
            updates,
            net_updates: net_effects.net_updates,
            load_deltas: net_effects.load_deltas,
            locality_updates: locality_effects.updates,
            metrics,
        }
    }

    pub(crate) fn evaluate_candidate_metrics<P>(
        &mut self,
        updates: &[(ClusterId, P)],
    ) -> PlacementMetrics
    where
        P: Copy + Into<Point>,
    {
        let updates = normalize_candidate_updates(updates);
        if updates.is_empty() {
            return self.metrics.clone();
        }

        let moved_clusters = moved_clusters(&updates);
        let net_metrics = self.candidate_net_metrics(&updates, &moved_clusters);
        let locality_cost = self.candidate_locality_cost(&updates, &moved_clusters);
        compose_metrics(
            self.mode,
            net_metrics.wire_cost,
            net_metrics.congestion_score_raw,
            net_metrics.timing_cost,
            locality_cost,
        )
    }

    pub(crate) fn apply_candidate(&mut self, candidate: PlacementCandidate) {
        for (cluster_id, position) in candidate.updates {
            if let Some(slot) = self.placements.get_mut(cluster_id.index()) {
                *slot = Some(position);
            }
        }

        for (index, delta) in candidate.load_deltas {
            if let Some(load) = self.loads.get_mut(index) {
                *load += delta;
            }
        }

        for (net_index, net_model) in candidate.net_updates {
            if let Some(slot) = self.net_models.get_mut(net_index) {
                *slot = net_model;
            }
        }

        for (cluster_id, locality_term) in candidate.locality_updates {
            if let Some(slot) = self.locality_terms.get_mut(cluster_id.index()) {
                *slot = locality_term;
            }
        }

        self.congestion_score_raw = candidate.metrics.congestion_cost / CONGESTION_SCALE;
        self.metrics = candidate.metrics;
    }

    fn empty_candidate(&self) -> PlacementCandidate {
        PlacementCandidate {
            updates: SmallVec::new(),
            net_updates: Vec::new(),
            load_deltas: Vec::new(),
            locality_updates: Vec::new(),
            metrics: self.metrics.clone(),
        }
    }

    fn candidate_net_effects(
        &mut self,
        updates: &ClusterUpdates,
        moved_clusters: &[ClusterId],
    ) -> CandidateNetEffects {
        self.collect_affected_nets(moved_clusters);
        let mut wire_cost = self.metrics.wire_cost;
        let mut timing_cost = self.metrics.timing_cost;
        let mut congestion_score_raw = self.congestion_score_raw;
        let affected_nets_len = self.scratch.affected_nets.len();
        let mut net_updates = Vec::with_capacity(affected_nets_len);

        for affected_index in 0..affected_nets_len {
            let net_index = self.scratch.affected_nets[affected_index];
            if let Some(previous) = self.net_models.get(net_index).and_then(Option::as_ref) {
                wire_cost -= previous.wire_cost();
                timing_cost -= previous.timing_cost();
                accumulate_load_delta(
                    previous,
                    self.arch,
                    -1.0,
                    &mut self.scratch.load_deltas,
                    &mut self.scratch.touched_loads,
                    &mut self.scratch.touched_mask,
                );
            }

            let next_model = self.model.nets.get(net_index).and_then(|net| {
                build_net_model_with_overrides(
                    net,
                    self.model,
                    &self.placements,
                    updates,
                    self.delay,
                    self.mode,
                )
            });
            if let Some(next) = next_model.as_ref() {
                wire_cost += next.wire_cost();
                timing_cost += next.timing_cost();
                accumulate_load_delta(
                    next,
                    self.arch,
                    1.0,
                    &mut self.scratch.load_deltas,
                    &mut self.scratch.touched_loads,
                    &mut self.scratch.touched_mask,
                );
            }
            net_updates.push((net_index, next_model));
        }

        for index in &self.scratch.touched_loads {
            let previous = self.loads.get(*index).copied().unwrap_or(0.0);
            let next = previous + self.scratch.load_deltas[*index];
            congestion_score_raw += overflow_score(next) - overflow_score(previous);
        }

        let load_deltas = self
            .scratch
            .touched_loads
            .iter()
            .copied()
            .filter_map(|index| {
                let delta = self.scratch.load_deltas[index];
                (delta.abs() > f64::EPSILON).then_some((index, delta))
            })
            .collect();

        self.clear_candidate_net_scratch();

        CandidateNetEffects {
            wire_cost,
            timing_cost,
            congestion_score_raw,
            load_deltas,
            net_updates,
        }
    }

    fn candidate_net_metrics(
        &mut self,
        updates: &ClusterUpdates,
        moved_clusters: &[ClusterId],
    ) -> CandidateNetMetrics {
        self.collect_affected_nets(moved_clusters);
        let mut wire_cost = self.metrics.wire_cost;
        let mut timing_cost = self.metrics.timing_cost;
        let mut congestion_score_raw = self.congestion_score_raw;
        let affected_nets_len = self.scratch.affected_nets.len();

        for affected_index in 0..affected_nets_len {
            let net_index = self.scratch.affected_nets[affected_index];
            if let Some(previous) = self.net_models.get(net_index).and_then(Option::as_ref) {
                wire_cost -= previous.wire_cost();
                timing_cost -= previous.timing_cost();
                accumulate_load_delta(
                    previous,
                    self.arch,
                    -1.0,
                    &mut self.scratch.load_deltas,
                    &mut self.scratch.touched_loads,
                    &mut self.scratch.touched_mask,
                );
            }

            let next_model = self.model.nets.get(net_index).and_then(|net| {
                build_net_model_with_overrides(
                    net,
                    self.model,
                    &self.placements,
                    updates,
                    self.delay,
                    self.mode,
                )
            });
            if let Some(next) = next_model.as_ref() {
                wire_cost += next.wire_cost();
                timing_cost += next.timing_cost();
                accumulate_load_delta(
                    next,
                    self.arch,
                    1.0,
                    &mut self.scratch.load_deltas,
                    &mut self.scratch.touched_loads,
                    &mut self.scratch.touched_mask,
                );
            }
        }

        for index in &self.scratch.touched_loads {
            let previous = self.loads.get(*index).copied().unwrap_or(0.0);
            let next = previous + self.scratch.load_deltas[*index];
            congestion_score_raw += overflow_score(next) - overflow_score(previous);
        }

        self.clear_candidate_net_scratch();

        CandidateNetMetrics {
            wire_cost,
            timing_cost,
            congestion_score_raw,
        }
    }

    fn candidate_locality_effects(
        &mut self,
        updates: &ClusterUpdates,
        moved_clusters: &[ClusterId],
    ) -> CandidateLocalityEffects {
        self.collect_affected_locality_clusters(moved_clusters);
        let mut cost = self.metrics.locality_cost;
        let affected_locality_len = self.scratch.affected_locality.len();
        let mut updates_out = Vec::with_capacity(affected_locality_len);

        for affected_index in 0..affected_locality_len {
            let cluster_id = self.scratch.affected_locality[affected_index];
            let previous = self
                .locality_terms
                .get(cluster_id.index())
                .copied()
                .unwrap_or(0.0);
            let next = locality_term(
                cluster_id,
                self.graph,
                &self.placements,
                updates,
                &self.locality_weights,
            )
            .unwrap_or(0.0);
            cost += next - previous;
            updates_out.push((cluster_id, next));
        }

        self.clear_candidate_locality_scratch();

        CandidateLocalityEffects {
            cost,
            updates: updates_out,
        }
    }

    fn candidate_locality_cost(
        &mut self,
        updates: &ClusterUpdates,
        moved_clusters: &[ClusterId],
    ) -> f64 {
        self.collect_affected_locality_clusters(moved_clusters);
        let mut cost = self.metrics.locality_cost;

        for affected_index in 0..self.scratch.affected_locality.len() {
            let cluster_id = self.scratch.affected_locality[affected_index];
            let previous = self
                .locality_terms
                .get(cluster_id.index())
                .copied()
                .unwrap_or(0.0);
            let next = locality_term(
                cluster_id,
                self.graph,
                &self.placements,
                updates,
                &self.locality_weights,
            )
            .unwrap_or(0.0);
            cost += next - previous;
        }

        self.clear_candidate_locality_scratch();
        cost
    }

    fn collect_affected_nets(&mut self, moved_clusters: &[ClusterId]) {
        self.scratch.affected_nets.clear();
        for cluster_id in moved_clusters {
            for net_index in self.model.nets_for_cluster(*cluster_id) {
                if let Some(slot) = self.scratch.affected_nets_seen.get_mut(*net_index)
                    && !*slot
                {
                    *slot = true;
                    self.scratch.affected_nets.push(*net_index);
                }
            }
        }
    }

    fn clear_candidate_net_scratch(&mut self) {
        for &index in &self.scratch.touched_loads {
            self.scratch.load_deltas[index] = 0.0;
            self.scratch.touched_mask[index] = false;
        }
        self.scratch.touched_loads.clear();
        for &net_index in &self.scratch.affected_nets {
            self.scratch.affected_nets_seen[net_index] = false;
        }
        self.scratch.affected_nets.clear();
    }

    fn collect_affected_locality_clusters(&mut self, moved_clusters: &[ClusterId]) {
        self.scratch.affected_locality.clear();
        for cluster_id in moved_clusters {
            self.mark_affected_locality_cluster(*cluster_id);
            for (neighbor, _) in self.graph.neighbors(*cluster_id) {
                self.mark_affected_locality_cluster(*neighbor);
            }
        }
    }

    fn mark_affected_locality_cluster(&mut self, cluster_id: ClusterId) {
        let slot = &mut self.scratch.affected_locality_seen[cluster_id.index()];
        if !*slot {
            *slot = true;
            self.scratch.affected_locality.push(cluster_id);
        }
    }

    fn clear_candidate_locality_scratch(&mut self) {
        for &cluster_id in &self.scratch.affected_locality {
            self.scratch.affected_locality_seen[cluster_id.index()] = false;
        }
        self.scratch.affected_locality.clear();
    }
}

impl PlacementCandidate {
    pub(crate) fn metrics(&self) -> &PlacementMetrics {
        &self.metrics
    }
}

#[cfg(test)]
pub(crate) fn evaluate(
    model: &PlacementModel,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    arch: &Arch,
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> PlacementMetrics {
    build_evaluation_state(model, graph, placements, arch, delay, mode)
        .metrics
        .clone()
}

pub(crate) fn evaluate_positions(
    model: &PlacementModel,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    arch: &Arch,
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> PlacementMetrics {
    build_evaluation_state(model, graph, placements, arch, delay, mode)
        .metrics
        .clone()
}

fn normalize_candidate_updates<P>(updates: &[(ClusterId, P)]) -> ClusterUpdates
where
    P: Copy + Into<Point>,
{
    updates
        .iter()
        .map(|(cluster_id, point)| (*cluster_id, (*point).into()))
        .collect()
}

fn moved_clusters(updates: &ClusterUpdates) -> SmallVec<[ClusterId; 2]> {
    updates.iter().map(|(cluster_id, _)| *cluster_id).collect()
}

fn build_evaluation_state(
    model: &PlacementModel,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    arch: &Arch,
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> EvaluationState {
    let net_models = build_net_models(model, placements, delay, mode);
    let mut wire_cost = 0.0;
    let mut timing_cost = 0.0;
    let mut loads = vec![0.0; arch.width.saturating_mul(arch.height).max(1)];

    for net_model in net_models.iter().flatten() {
        wire_cost += net_model.wire_cost();
        timing_cost += net_model.timing_cost();
        apply_net_load(net_model, arch, 1.0, &mut loads);
    }

    let congestion_score_raw = loads.iter().copied().map(overflow_score).sum::<f64>();
    let locality_weights = (0..model.cluster_count())
        .map(|index| graph.total_weight(ClusterId::new(index)))
        .collect::<Vec<_>>();
    let locality_terms = (0..model.cluster_count())
        .map(|index| {
            locality_term(
                ClusterId::new(index),
                graph,
                placements,
                &[],
                &locality_weights,
            )
            .unwrap_or(0.0)
        })
        .collect::<Vec<_>>();
    let locality_cost = locality_terms.iter().sum::<f64>();
    let metrics = compose_metrics(
        mode,
        wire_cost,
        congestion_score_raw,
        timing_cost,
        locality_cost,
    );

    EvaluationState {
        net_models,
        loads,
        locality_terms,
        locality_weights,
        congestion_score_raw,
        metrics,
    }
}

fn build_net_models(
    model: &PlacementModel,
    placements: &[Option<Point>],
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> Vec<Option<NetModel>> {
    if model.nets.len() >= PARALLEL_NET_THRESHOLD {
        model
            .nets
            .par_iter()
            .map(|net| build_net_model(net, model, placements, delay, mode))
            .collect::<Vec<_>>()
    } else {
        model
            .nets
            .iter()
            .map(|net| build_net_model(net, model, placements, delay, mode))
            .collect::<Vec<_>>()
    }
}

fn locality_term(
    cluster_id: ClusterId,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    overrides: &[(ClusterId, Point)],
    locality_weights: &[f64],
) -> Option<f64> {
    let position = lookup_position(cluster_id, placements, overrides)?;
    let centroid = weighted_centroid_with_overrides(cluster_id, graph, placements, overrides)?;
    let weight = locality_weights
        .get(cluster_id.index())
        .copied()
        .unwrap_or(0.0);
    Some(0.08 * weight * manhattan(position, centroid) as f64)
}

fn weighted_centroid_with_overrides(
    cluster_id: ClusterId,
    graph: &ClusterGraph,
    placements: &[Option<Point>],
    overrides: &[(ClusterId, Point)],
) -> Option<Point> {
    let mut x_total = 0.0;
    let mut y_total = 0.0;
    let mut weight_total = 0.0;

    for (neighbor, weight) in graph.neighbors(cluster_id) {
        let point = lookup_position(*neighbor, placements, overrides)?;
        x_total += point.x as f64 * weight;
        y_total += point.y as f64 * weight;
        weight_total += weight;
    }

    if weight_total == 0.0 {
        None
    } else {
        Some(Point::new(
            (x_total / weight_total).round() as usize,
            (y_total / weight_total).round() as usize,
        ))
    }
}

fn lookup_position(
    cluster_id: ClusterId,
    placements: &[Option<Point>],
    overrides: &[(ClusterId, Point)],
) -> Option<Point> {
    overrides
        .iter()
        .rev()
        .find(|(candidate, _)| *candidate == cluster_id)
        .map(|(_, point)| *point)
        .or_else(|| placements.get(cluster_id.index()).copied().flatten())
}

fn compose_metrics(
    mode: PlaceMode,
    wire_cost: f64,
    congestion_score_raw: f64,
    timing_cost: f64,
    locality_cost: f64,
) -> PlacementMetrics {
    let congestion_cost = congestion_score_raw * CONGESTION_SCALE;
    let total = match mode {
        PlaceMode::BoundingBox => wire_cost + 0.75 * congestion_cost + 0.50 * locality_cost,
        PlaceMode::TimingDriven => {
            wire_cost + 1.15 * congestion_cost + 1.35 * timing_cost + 0.75 * locality_cost
        }
    };

    PlacementMetrics {
        wire_cost,
        congestion_cost,
        timing_cost,
        locality_cost,
        total,
    }
}

fn overflow_score(load: f64) -> f64 {
    let overflow = (load - CONGESTION_THRESHOLD).max(0.0);
    overflow * overflow
}

fn accumulate_load_delta(
    net_model: &NetModel,
    arch: &Arch,
    scale: f64,
    deltas: &mut [f64],
    touched: &mut Vec<usize>,
    touched_mask: &mut [bool],
) {
    let cell_load = net_model.cell_load() * scale;
    for x in net_model.min_x..=net_model.max_x {
        for y in net_model.min_y..=net_model.max_y {
            let index = y * arch.width + x;
            if !touched_mask[index] {
                touched_mask[index] = true;
                touched.push(index);
            }
            deltas[index] += cell_load;
        }
    }
}

fn apply_net_load(net_model: &NetModel, arch: &Arch, scale: f64, loads: &mut [f64]) {
    let cell_load = net_model.cell_load() * scale;
    for x in net_model.min_x..=net_model.max_x {
        for y in net_model.min_y..=net_model.max_y {
            let index = y * arch.width + x;
            if let Some(load) = loads.get_mut(index) {
                *load += cell_load;
            }
        }
    }
}

fn build_net_model(
    net: &PreparedNet,
    model: &PlacementModel,
    placements: &[Option<Point>],
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> Option<NetModel> {
    build_net_model_with_overrides(net, model, placements, &[], delay, mode)
}

fn build_net_model_with_overrides(
    net: &PreparedNet,
    model: &PlacementModel,
    placements: &[Option<Point>],
    overrides: &[(ClusterId, Point)],
    delay: Option<&DelayModel>,
    mode: PlaceMode,
) -> Option<NetModel> {
    let driver = net.driver?;
    let src = model.point_for_overrides(driver, placements, overrides)?;
    let mut min_x = src.x;
    let mut max_x = src.x;
    let mut min_y = src.y;
    let mut max_y = src.y;
    let mut connected_points = 1usize;
    let mut driver_span = 0.0_f64;

    for sink in &net.sinks {
        if let Some(point) = model.point_for_overrides(*sink, placements, overrides) {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
            connected_points += 1;
            driver_span = driver_span.max(manhattan(src, point) as f64);
        }
    }
    if connected_points <= 1 {
        return None;
    }

    let dx = max_x - min_x;
    let dy = max_y - min_y;
    let hpwl = (dx + dy) as f64;
    let route_delay = delay
        .map(|table| table.lookup(dx, dy))
        .unwrap_or(hpwl * 0.08);
    let fanout = net.fanout as f64;
    let base_weight = 1.0 + 0.12 * fanout.min(8.0);
    let weight = match mode {
        PlaceMode::BoundingBox => base_weight,
        PlaceMode::TimingDriven => base_weight + 1.4 * net.criticality.max(0.0),
    };

    Some(NetModel {
        min_x,
        max_x,
        min_y,
        max_y,
        area: (dx + 1) * (dy + 1),
        hpwl,
        route_delay,
        driver_span,
        weight,
    })
}

impl NetModel {
    fn cell_load(&self) -> f64 {
        self.weight / self.area.max(1) as f64
    }

    fn wire_cost(&self) -> f64 {
        let span_area = (self.max_x - self.min_x + 1) * (self.max_y - self.min_y + 1);
        self.weight * (self.hpwl + 0.35 * self.route_delay + 0.08 * (span_area as f64).sqrt())
    }

    fn timing_cost(&self) -> f64 {
        self.weight * (self.route_delay + 0.12 * self.driver_span)
    }
}

impl EvaluationScratch {
    fn new(model: &PlacementModel, arch: &Arch) -> Self {
        let load_count = arch.width.saturating_mul(arch.height).max(1);
        let cluster_count = model.cluster_count().max(1);
        Self {
            affected_nets_seen: vec![false; model.nets.len()],
            affected_nets: Vec::new(),
            load_deltas: vec![0.0; load_count],
            touched_loads: Vec::new(),
            touched_mask: vec![false; load_count],
            affected_locality_seen: vec![false; cluster_count],
            affected_locality: Vec::new(),
        }
    }
}
