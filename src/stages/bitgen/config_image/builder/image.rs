use std::collections::{BTreeMap, BTreeSet};

use super::super::accumulator::TileAccumulator;
use super::super::{ConfigImage, TileBitAssignment};
use super::target::{SourceTileContext, TargetAssignment};

pub(super) struct ConfigImageBuilder {
    notes: Vec<String>,
    tiles: BTreeMap<(String, String, usize, usize), TileAccumulator>,
}

impl ConfigImageBuilder {
    pub(super) fn new() -> Self {
        Self {
            notes: vec![
                "Rust tile config image covers logic/IO/clock site SRAM and routed transmission SRAM when available."
                    .to_string(),
            ],
            tiles: BTreeMap::new(),
        }
    }

    pub(super) fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }

    pub(super) fn extend_notes(&mut self, notes: impl IntoIterator<Item = String>) {
        self.notes.extend(notes);
    }

    pub(super) fn register_config(
        &mut self,
        source: SourceTileContext<'_>,
        site_name: &str,
        cfg_name: &str,
        function_name: &str,
    ) {
        self.tile_mut(
            source.tile_name,
            source.tile_type,
            source.x,
            source.y,
            source.rows,
            source.cols,
        )
        .configs_mut()
        .insert((
            site_name.to_string(),
            cfg_name.to_string(),
            function_name.to_string(),
        ));
    }

    pub(super) fn insert_assignment(
        &mut self,
        target: &TargetAssignment,
        assignment: TileBitAssignment,
    ) {
        self.tile_mut(
            &target.tile_name,
            &target.tile_type,
            target.x,
            target.y,
            target.rows,
            target.cols,
        )
        .insert(assignment);
    }

    pub(super) fn finish(self) -> ConfigImage {
        let mut image = ConfigImage {
            tiles: self
                .tiles
                .into_values()
                .map(TileAccumulator::finish)
                .filter(|tile| !tile.configs.is_empty() || !tile.assignments.is_empty())
                .collect(),
            notes: self.notes,
        };
        let mut unique_notes = BTreeSet::new();
        image.notes.retain(|note| unique_notes.insert(note.clone()));
        image.tiles.sort_by(|lhs, rhs| {
            (lhs.y, lhs.x, lhs.tile_name.as_str()).cmp(&(rhs.y, rhs.x, rhs.tile_name.as_str()))
        });
        image
    }

    fn tile_mut(
        &mut self,
        tile_name: &str,
        tile_type: &str,
        x: usize,
        y: usize,
        rows: usize,
        cols: usize,
    ) -> &mut TileAccumulator {
        self.tiles
            .entry((tile_name.to_string(), tile_type.to_string(), x, y))
            .or_insert_with(|| TileAccumulator::new_tile(tile_name, tile_type, x, y, rows, cols))
    }
}
