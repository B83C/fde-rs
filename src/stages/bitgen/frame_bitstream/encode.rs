mod memory;

use crate::cil::Cil;
use anyhow::{Result, anyhow, bail};

use super::model::{MajorPayload, TileColumns};

pub(crate) use memory::build_memory_payloads;

pub(crate) fn build_major_payloads(cil: &Cil, columns: &TileColumns) -> Result<Vec<MajorPayload>> {
    let bits_per_group_reversed = parameter_usize(cil, "bits_per_grp_reversed")?;
    let initial_num = parameter_bit(cil, "initialNum")?;

    let mut payloads = Vec::with_capacity(cil.majors.len());
    for major in &cil.majors {
        let Some(column_tiles) = columns.get(&major.tile_col) else {
            bail!(
                "missing bitstream column {} required by major {}",
                major.tile_col,
                major.address
            );
        };

        let mut words = Vec::new();
        let mut expected_frame_bits = None;
        for frame_index in 0..major.frame_count {
            let mut frame_bits =
                collect_frame_bits(column_tiles, major.tile_col, major.address, frame_index)?;
            let frame_len = frame_bits.len();
            if let Some(expected_len) = expected_frame_bits {
                if expected_len != frame_len {
                    bail!(
                        "major {} frame {} has {} bits, expected {}",
                        major.address,
                        frame_index,
                        frame_len,
                        expected_len
                    );
                }
            } else {
                expected_frame_bits = Some(frame_len);
            }

            reverse_groups(&mut frame_bits, bits_per_group_reversed);
            words.extend(bits_to_words(&mut frame_bits, initial_num));
        }

        payloads.push(MajorPayload {
            address: major.address,
            words,
        });
    }

    Ok(payloads)
}

pub(crate) fn parameter_usize(cil: &Cil, name: &str) -> Result<usize> {
    cil.bitstream_parameters
        .get(name)
        .ok_or_else(|| anyhow!("missing bitstream parameter {name}"))?
        .trim()
        .parse::<usize>()
        .map_err(|error| anyhow!("invalid bitstream parameter {name}: {error}"))
}

pub(crate) fn parameter_u32(cil: &Cil, name: &str) -> Result<u32> {
    let value = parameter_usize(cil, name)?;
    u32::try_from(value).map_err(|_| anyhow!("bitstream parameter {name} is too large"))
}

fn parameter_bit(cil: &Cil, name: &str) -> Result<u8> {
    let value = parameter_usize(cil, name)?;
    match value {
        0 => Ok(0),
        1 => Ok(1),
        _ => bail!("bitstream parameter {name} must be 0 or 1, got {value}"),
    }
}

fn collect_frame_bits(
    column_tiles: &[super::model::TileFrameImage],
    tile_col: usize,
    major_address: usize,
    frame_index: usize,
) -> Result<Vec<u8>> {
    let mut frame_bits = Vec::new();
    for tile in column_tiles {
        if frame_index >= tile.cols {
            bail!(
                "tile {} ({}) in column {} exposes only {} frames, but major {} requests frame {}",
                tile.tile_name,
                tile.tile_type,
                tile_col,
                tile.cols,
                major_address,
                frame_index
            );
        }
        for row in 0..tile.rows {
            frame_bits.push(tile.bits[row * tile.cols + frame_index]);
        }
    }
    Ok(frame_bits)
}

pub(crate) fn reverse_groups(bits: &mut [u8], group_size: usize) {
    if group_size <= 1 {
        return;
    }
    for chunk in bits.chunks_exact_mut(group_size) {
        chunk.reverse();
    }
}

pub(crate) fn bits_to_words(bits: &mut Vec<u8>, fill_bit: u8) -> Vec<u32> {
    let normalized_fill_bit = fill_bit & 1;
    let words = bits.len().div_ceil(32);
    bits.resize(words * 32, normalized_fill_bit);
    bits.chunks(32)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0u32, |word, bit| (word << 1) | u32::from(bit & 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{bits_to_words, reverse_groups};

    #[test]
    fn reverses_only_complete_groups() {
        let mut bits = vec![0, 1, 1, 0, 1, 0, 0, 1, 1];
        reverse_groups(&mut bits, 4);
        assert_eq!(bits, vec![0, 1, 1, 0, 1, 0, 0, 1, 1]);

        let mut bits = vec![0, 1, 1, 1, 1, 0, 0, 0];
        reverse_groups(&mut bits, 4);
        assert_eq!(bits, vec![1, 1, 1, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn packs_bits_into_msb_first_words() {
        let mut bits = vec![1, 0, 1, 0, 1, 0, 1, 0];
        let words = bits_to_words(&mut bits, 1);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0xaaff_ffff);
    }
}
