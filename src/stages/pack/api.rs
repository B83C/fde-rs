use super::DEFAULT_PACK_CAPACITY;
use crate::{
    domain::{ClusterKind, SequentialCellType},
    ir::{CellId, Cluster, Design, DesignIndex},
    report::{StageOutput, StageReport, StageReporter, emit_stage_info},
};
use anyhow::Result;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug, Clone)]
pub struct PackOptions {
    pub family: Option<String>,
    pub capacity: usize,
    pub cell_library: Option<PathBuf>,
    pub dcp_library: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            family: None,
            capacity: DEFAULT_PACK_CAPACITY,
            cell_library: None,
            dcp_library: None,
            config: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ControlSet {
    clock_net: Option<String>,
    clock_inverted: bool,
    clock_enable_net: Option<String>,
    set_reset_net: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LaneKind {
    LutFf,
    Lut,
    Ff,
    Other,
}

#[derive(Debug, Clone)]
struct Lane {
    kind: LaneKind,
    members: Vec<CellId>,
    control_set: Option<ControlSet>,
}

#[derive(Debug, Clone)]
struct LaneTopology {
    share: Vec<Vec<usize>>,
    conn: Vec<Vec<usize>>,
    degree: Vec<usize>,
    conn_factor: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct LaneUsage {
    members: usize,
    luts: usize,
    ffs: usize,
}

#[derive(Debug, Clone)]
struct ClusterPlan {
    kind: ClusterKind,
    members: Vec<CellId>,
    capacity: usize,
}

pub fn run(design: Design, options: &PackOptions) -> Result<StageOutput<Design>> {
    run_internal(design, options, None)
}

pub fn run_with_reporter(
    design: Design,
    options: &PackOptions,
    reporter: &mut dyn StageReporter,
) -> Result<StageOutput<Design>> {
    run_internal(design, options, Some(reporter))
}

fn run_internal(
    mut design: Design,
    options: &PackOptions,
    mut reporter: Option<&mut dyn StageReporter>,
) -> Result<StageOutput<Design>> {
    let capacity = options.capacity.max(2);
    design.stage = "packed".to_string();
    emit_stage_info(
        &mut reporter,
        "pack",
        format!(
            "packing {} cells into clusters with capacity {}",
            design.cells.len(),
            capacity
        ),
    );
    if let Some(family) = &options.family {
        design.metadata.family = family.clone();
        emit_stage_info(
            &mut reporter,
            "pack",
            format!("targeting family {}", family),
        );
    }
    if let Some(cell_library) = &options.cell_library {
        design.note_once(format!(
            "Pack referenced cell library {}",
            cell_library.display()
        ));
        emit_stage_info(
            &mut reporter,
            "pack",
            format!("using pack cell library {}", cell_library.display()),
        );
    }
    if let Some(dcp_library) = &options.dcp_library {
        design.note_once(format!(
            "Pack referenced dc library {}",
            dcp_library.display()
        ));
        emit_stage_info(
            &mut reporter,
            "pack",
            format!("using pack DCP library {}", dcp_library.display()),
        );
    }
    if let Some(config) = &options.config {
        design.note_once(format!("Pack referenced config {}", config.display()));
        emit_stage_info(
            &mut reporter,
            "pack",
            format!("using pack config {}", config.display()),
        );
    }

    let cluster_plans = build_cluster_plans(&design, capacity);
    let block_ram_cluster_count = cluster_plans
        .iter()
        .filter(|plan| plan.kind == ClusterKind::BlockRam)
        .count();
    emit_stage_info(
        &mut reporter,
        "pack",
        format!(
            "built {} cluster plans ({} block RAM, {} logic)",
            cluster_plans.len(),
            block_ram_cluster_count,
            cluster_plans.len().saturating_sub(block_ram_cluster_count)
        ),
    );
    let clusters = cluster_plans
        .iter()
        .enumerate()
        .map(|(cluster_index, plan)| {
            Cluster::new(next_cluster_name(plan.kind, cluster_index), plan.kind)
                .with_members(cell_names(&design, &plan.members))
                .with_capacity(plan.capacity)
        })
        .collect::<Vec<_>>();

    for (cluster_index, plan) in cluster_plans.iter().enumerate() {
        for cell_id in &plan.members {
            design.cells[cell_id.index()].cluster = Some(clusters[cluster_index].name.clone());
        }
    }

    design.clusters = clusters;
    emit_stage_info(
        &mut reporter,
        "pack",
        format!(
            "assigned {} cells across {} clusters",
            design.cells.len(),
            design.clusters.len()
        ),
    );
    let mut report = StageReport::new("pack");
    report.metric("logical_cell_count", design.cells.len());
    report.metric("cluster_count", design.clusters.len());
    report.metric("cluster_capacity", capacity);
    report.metric(
        "average_cluster_fill",
        if design.clusters.is_empty() {
            0.0
        } else {
            design.cells.len() as f64 / design.clusters.len() as f64
        },
    );
    report.push(format!(
        "Packed {} logical cells into {} clusters (capacity {}).",
        design.cells.len(),
        design.clusters.len(),
        capacity
    ));

    Ok(StageOutput {
        value: design,
        report,
    })
}

fn build_cluster_plans(design: &Design, capacity: usize) -> Vec<ClusterPlan> {
    let index = design.index();
    let ordered_cells = ordered_cell_ids(design);
    let mut plans = ordered_cells
        .iter()
        .copied()
        .filter(|cell_id| index.cell(design, *cell_id).is_block_ram())
        .map(|cell_id| ClusterPlan {
            kind: ClusterKind::BlockRam,
            members: vec![cell_id],
            capacity: 1,
        })
        .collect::<Vec<_>>();
    let lanes = build_lanes(design, &index);
    let topology = build_lane_topology(design, &index, &lanes);
    let mut seed_lanes = (0..lanes.len()).collect::<Vec<_>>();
    seed_lanes.sort_by(|lhs, rhs| compare_seed_lanes(lhs, rhs, &topology));

    let mut used = vec![false; lanes.len()];
    let mut clusters = Vec::<Vec<CellId>>::new();
    for seed in seed_lanes {
        if used[seed] {
            continue;
        }
        if lanes[seed].kind == LaneKind::Other {
            used[seed] = true;
            clusters.push(flatten_cluster_lanes(&lanes, &[seed]));
            continue;
        }

        used[seed] = true;
        let mut cluster_lane_ids = vec![seed];
        let mut usage = lane_usage(&lanes[seed]);
        let mut control_set = lanes[seed].control_set.clone();

        while cluster_lane_ids.len() < 2 {
            let Some(next) = best_cluster_candidate(
                &lanes,
                &topology,
                &used,
                &cluster_lane_ids,
                usage,
                control_set.as_ref(),
                capacity,
            ) else {
                break;
            };
            used[next] = true;
            usage = merge_lane_usage(usage, lane_usage(&lanes[next]));
            if control_set.is_none() {
                control_set = lanes[next].control_set.clone();
            }
            cluster_lane_ids.push(next);
        }

        clusters.push(flatten_cluster_lanes(&lanes, &cluster_lane_ids));
    }

    plans.extend(clusters.into_iter().map(|members| ClusterPlan {
        kind: ClusterKind::Logic,
        members,
        capacity,
    }));
    plans
}

fn build_lanes(design: &Design, index: &DesignIndex<'_>) -> Vec<Lane> {
    let mut used = vec![false; design.cells.len()];
    let mut lanes = Vec::new();
    let ordered_cells = ordered_cell_ids(design);

    for ff_id in ordered_cells.iter().copied() {
        let cell = index.cell(design, ff_id);
        if used[ff_id.index()] || !cell.is_sequential() {
            continue;
        }
        let control_set = Some(control_set_for_cell(cell));
        if let Some(lut_id) = paired_lut_for_ff(design, index, ff_id, &used) {
            used[lut_id.index()] = true;
            used[ff_id.index()] = true;
            lanes.push(make_lane(LaneKind::LutFf, vec![lut_id, ff_id], control_set));
        } else {
            used[ff_id.index()] = true;
            lanes.push(make_lane(LaneKind::Ff, vec![ff_id], control_set));
        }
    }

    for cell_id in ordered_cells {
        let cell = index.cell(design, cell_id);
        if used[cell_id.index()] || cell.is_constant_source() || cell.is_block_ram() {
            continue;
        }
        used[cell_id.index()] = true;
        lanes.push(make_lane(
            if cell.is_lut() {
                LaneKind::Lut
            } else {
                LaneKind::Other
            },
            vec![cell_id],
            None,
        ));
    }

    lanes
}

fn make_lane(kind: LaneKind, members: Vec<CellId>, control_set: Option<ControlSet>) -> Lane {
    Lane {
        kind,
        members,
        control_set,
    }
}

fn build_lane_topology(design: &Design, index: &DesignIndex<'_>, lanes: &[Lane]) -> LaneTopology {
    let mut share = vec![vec![0usize; lanes.len()]; lanes.len()];
    let mut conn = vec![vec![0usize; lanes.len()]; lanes.len()];
    let mut cell_to_lane = vec![None; design.cells.len()];
    let mut degree = vec![0usize; lanes.len()];
    let mut terminal_count = vec![0usize; lanes.len()];
    for (lane_index, lane) in lanes.iter().enumerate() {
        for member in &lane.members {
            cell_to_lane[member.index()] = Some(lane_index);
        }
    }

    for net in &design.nets {
        let mut touched_lanes = BTreeSet::new();
        let mut touched_pins = 0usize;
        let mut driver_lane = None;
        if let Some(driver) = net.driver.as_ref()
            && let Some(cell_id) = index.cell_for_endpoint(driver)
            && !index.cell(design, cell_id).is_constant_source()
        {
            let Some(lane) = cell_to_lane[cell_id.index()] else {
                continue;
            };
            touched_lanes.insert(lane);
            driver_lane = Some(lane);
            touched_pins += 1;
        }
        for sink in &net.sinks {
            let Some(cell_id) = index.cell_for_endpoint(sink) else {
                continue;
            };
            let Some(lane) = cell_to_lane[cell_id.index()] else {
                continue;
            };
            touched_lanes.insert(lane);
            touched_pins += 1;
        }
        let touched = touched_lanes.into_iter().collect::<Vec<_>>();
        for &lane in &touched {
            degree[lane] += 1;
            terminal_count[lane] += touched_pins;
        }
        for (offset, &lhs) in touched.iter().enumerate() {
            for &rhs in &touched[offset + 1..] {
                share[lhs][rhs] += 1;
                share[rhs][lhs] += 1;
            }
        }
        if let Some(driver_lane) = driver_lane {
            for sink in &net.sinks {
                let Some(cell_id) = index.cell_for_endpoint(sink) else {
                    continue;
                };
                let Some(sink_lane) = cell_to_lane[cell_id.index()] else {
                    continue;
                };
                if sink_lane == driver_lane {
                    continue;
                }
                conn[driver_lane][sink_lane] += 1;
                conn[sink_lane][driver_lane] += 1;
            }
        }
    }

    let conn_factor = degree
        .iter()
        .enumerate()
        .map(|(lane, degree)| {
            if *degree == 0 {
                0.0
            } else {
                terminal_count[lane] as f64 / ((*degree * *degree) as f64)
            }
        })
        .collect::<Vec<_>>();

    LaneTopology {
        share,
        conn,
        degree,
        conn_factor,
    }
}

fn compare_seed_lanes(lhs: &usize, rhs: &usize, topology: &LaneTopology) -> std::cmp::Ordering {
    topology.degree[*rhs]
        .cmp(&topology.degree[*lhs])
        .then_with(|| {
            topology.conn_factor[*lhs]
                .partial_cmp(&topology.conn_factor[*rhs])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.cmp(rhs))
}

fn lane_usage(lane: &Lane) -> LaneUsage {
    match lane.kind {
        LaneKind::LutFf => LaneUsage {
            members: lane.members.len(),
            luts: 1,
            ffs: 1,
        },
        LaneKind::Lut => LaneUsage {
            members: lane.members.len(),
            luts: 1,
            ffs: 0,
        },
        LaneKind::Ff => LaneUsage {
            members: lane.members.len(),
            luts: 0,
            ffs: 1,
        },
        LaneKind::Other => LaneUsage {
            members: lane.members.len(),
            luts: 0,
            ffs: 0,
        },
    }
}

fn merge_lane_usage(lhs: LaneUsage, rhs: LaneUsage) -> LaneUsage {
    LaneUsage {
        members: lhs.members + rhs.members,
        luts: lhs.luts + rhs.luts,
        ffs: lhs.ffs + rhs.ffs,
    }
}

fn best_cluster_candidate(
    lanes: &[Lane],
    topology: &LaneTopology,
    used: &[bool],
    cluster_lane_ids: &[usize],
    usage: LaneUsage,
    cluster_control_set: Option<&ControlSet>,
    capacity: usize,
) -> Option<usize> {
    let cluster_kind = lanes[cluster_lane_ids[0]].kind;
    let mut best_connected = None::<((usize, usize), usize)>;
    let mut best_unconnected_lut = None::<usize>;
    for candidate in 0..lanes.len() {
        if used[candidate] || lanes[candidate].kind == LaneKind::Other {
            continue;
        }
        if !cluster_can_accept_lane(
            cluster_kind,
            cluster_lane_ids,
            &lanes[candidate],
            usage,
            cluster_control_set,
            capacity,
        ) {
            continue;
        }
        let share = cluster_lane_ids
            .iter()
            .map(|lane_id| topology.share[*lane_id][candidate])
            .sum::<usize>();
        let conn = cluster_lane_ids
            .iter()
            .map(|lane_id| topology.conn[*lane_id][candidate])
            .sum::<usize>();
        if share != 0 || conn != 0 {
            let score = (share + conn, conn);
            match best_connected {
                Some((best_score, best_lane))
                    if score < best_score
                        || (score == best_score
                            && compare_seed_lanes(&candidate, &best_lane, topology)
                                == std::cmp::Ordering::Greater) => {}
                _ => best_connected = Some((score, candidate)),
            }
            continue;
        }
        if cluster_kind == LaneKind::Lut {
            match best_unconnected_lut {
                Some(best_lane)
                    if compare_seed_lanes(&candidate, &best_lane, topology)
                        == std::cmp::Ordering::Greater => {}
                _ => best_unconnected_lut = Some(candidate),
            }
        }
    }
    best_connected
        .map(|(_, lane_id)| lane_id)
        .or(best_unconnected_lut)
}

fn cluster_can_accept_lane(
    cluster_kind: LaneKind,
    cluster_lane_ids: &[usize],
    candidate: &Lane,
    usage: LaneUsage,
    cluster_control_set: Option<&ControlSet>,
    capacity: usize,
) -> bool {
    if cluster_lane_ids.len() >= 2 {
        return false;
    }
    if candidate.kind != cluster_kind {
        return false;
    }
    let merged = merge_lane_usage(usage, lane_usage(candidate));
    if merged.members > capacity || merged.luts > 2 || merged.ffs > 2 {
        return false;
    }
    match (cluster_control_set, candidate.control_set.as_ref()) {
        (Some(existing), Some(candidate)) => existing == candidate,
        _ => true,
    }
}

fn paired_lut_for_ff(
    design: &Design,
    index: &DesignIndex<'_>,
    ff_id: CellId,
    used: &[bool],
) -> Option<CellId> {
    let ff = index.cell(design, ff_id);
    let d_pin = ff
        .inputs
        .iter()
        .find(|pin| ff.primitive_kind().is_register_data_pin(&pin.port))?;
    let net_id = index.net_id(&d_pin.net)?;
    let net = index.net(design, net_id);
    let driver = net.driver.as_ref()?;
    let lut_id = index.cell_for_endpoint(driver)?;
    let lut = index.cell(design, lut_id);
    if used[lut_id.index()] || !lut.is_lut() {
        return None;
    }
    Some(lut_id)
}

fn control_set_for_cell(cell: &crate::ir::Cell) -> ControlSet {
    ControlSet {
        clock_net: cell.register_clock_net().map(str::to_string),
        clock_inverted: cell.register_clock_is_inverted(),
        clock_enable_net: cell.register_clock_enable_net().map(str::to_string),
        set_reset_net: cell.register_set_reset_net().map(str::to_string),
    }
}

fn flatten_cluster_lanes(lanes: &[Lane], lane_ids: &[usize]) -> Vec<CellId> {
    let mut members = Vec::new();
    for lane_id in lane_ids {
        members.extend(lanes[*lane_id].members.iter().copied());
    }
    members
}

fn ordered_cell_ids(design: &Design) -> Vec<CellId> {
    let mut cell_ids = (0..design.cells.len()).map(CellId::new).collect::<Vec<_>>();
    cell_ids.sort_by_key(|cell_id| {
        let cell = &design.cells[cell_id.index()];
        (pack_class_rank(cell), pack_rule_rank(cell), cell_id.index())
    });
    cell_ids
}

fn pack_class_rank(cell: &crate::ir::Cell) -> u8 {
    if cell.is_block_ram() {
        0
    } else if cell.is_sequential() {
        1
    } else if cell.is_lut() {
        2
    } else {
        3
    }
}

fn pack_rule_rank(cell: &crate::ir::Cell) -> u16 {
    if cell.is_block_ram() {
        0
    } else if cell.is_lut() {
        lut_rule_rank(&cell.type_name)
    } else if cell.is_sequential() {
        ff_rule_rank(&cell.type_name)
    } else {
        u16::MAX
    }
}

fn lut_rule_rank(type_name: &str) -> u16 {
    match type_name {
        "LUT1" => 0,
        "LUT2" => 1,
        "LUT3" => 2,
        "LUT4" => 3,
        _ => u16::MAX,
    }
}

fn ff_rule_rank(type_name: &str) -> u16 {
    match SequentialCellType::from_type_name(type_name) {
        Some(SequentialCellType::DffHq) => 0,
        Some(SequentialCellType::EdffHq) => 1,
        _ => u16::MAX,
    }
}

fn next_cluster_name(kind: ClusterKind, index: usize) -> String {
    match kind {
        ClusterKind::Logic => format!("clb_{index:04}"),
        ClusterKind::BlockRam => format!("bram_{index:04}"),
        ClusterKind::Unknown => format!("cluster_{index:04}"),
    }
}

fn cell_names(design: &Design, members: &[CellId]) -> Vec<String> {
    members
        .iter()
        .map(|member| design.cells[member.index()].name.clone())
        .collect()
}
