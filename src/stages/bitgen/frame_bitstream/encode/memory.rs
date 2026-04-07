use super::parameter_usize;
use crate::{
    bitgen::ProgrammingImage,
    cil::Cil,
    resource::{Arch, TileInstance},
};
use anyhow::{Result, anyhow, bail};

use super::super::model::DEFAULT_MEM_WORDS;

pub(crate) fn build_memory_payloads(
    cil: &Cil,
    arch: &Arch,
    programming_image: Option<&ProgrammingImage>,
) -> Result<Vec<Vec<u32>>> {
    let mem_amount = parameter_usize(cil, "mem_amount")?;
    let mut chunks = vec![vec![0u32; DEFAULT_MEM_WORDS]; mem_amount];
    let Some(programming_image) = programming_image else {
        return Ok(chunks);
    };
    if programming_image.memories.is_empty() {
        return Ok(chunks);
    }

    let chunk_indices = memory_chunk_indices(arch);
    for memory in &programming_image.memories {
        let Some(&chunk_index) = chunk_indices.get(memory.tile_name.as_str()) else {
            bail!(
                "block RAM tile {} is not mapped to any textual memory chunk",
                memory.tile_name
            );
        };
        if chunk_index >= chunks.len() {
            bail!(
                "block RAM tile {} resolved to memory chunk {} beyond mem_amount {}",
                memory.tile_name,
                chunks.len(),
                chunk_index,
            );
        }

        apply_memory_init_words(&mut chunks[chunk_index], &memory.init_words).map_err(|error| {
            anyhow!("failed to encode {} memory init: {error}", memory.tile_name)
        })?;
    }

    Ok(chunks)
}

fn memory_chunk_indices(arch: &Arch) -> std::collections::BTreeMap<&str, usize> {
    let mut ordered_tiles = arch
        .tiles
        .values()
        .filter(|tile| is_memory_chunk_tile(tile))
        .map(|tile| (tile.bit_y, tile.bit_x, tile.name.as_str()))
        .collect::<Vec<_>>();
    ordered_tiles.sort_unstable();
    ordered_tiles
        .into_iter()
        .enumerate()
        .map(|(index, (_, _, name))| (name, index))
        .collect()
}

fn is_memory_chunk_tile(tile: &TileInstance) -> bool {
    tile.kind().is_block_ram()
        && (tile.name.starts_with("BRAMR")
            || tile.tile_type.eq_ignore_ascii_case("LBRAMD")
            || tile.tile_type.eq_ignore_ascii_case("RBRAMD"))
}

fn apply_memory_init_words(chunk: &mut [u32], init_words: &[(String, String)]) -> Result<()> {
    for (cfg_name, raw_value) in init_words {
        let Some(init_index) = parse_init_index(cfg_name) else {
            continue;
        };
        let start = init_index * 8;
        if start + 8 > chunk.len() {
            bail!(
                "{} exceeds the {}-word BRAM textual payload",
                cfg_name,
                chunk.len()
            );
        }
        let words = parse_init_payload_words(raw_value)?;
        chunk[start..start + 8].copy_from_slice(&words);
    }
    Ok(())
}

fn parse_init_index(cfg_name: &str) -> Option<usize> {
    let suffix = cfg_name.strip_prefix("INIT_")?;
    if suffix.len() != 2 || !suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    usize::from_str_radix(suffix, 16).ok()
}

fn parse_init_payload_words(raw_value: &str) -> Result<[u32; 8]> {
    if let Some(binary) = parse_binary_literal(raw_value)? {
        return binary_words_to_u32s(&binary);
    }

    let hex = parse_hex_literal(raw_value)?;
    hex_words_to_u32s(&hex)
}

fn parse_binary_literal(raw_value: &str) -> Result<Option<String>> {
    let trimmed = raw_value.trim();
    if let Some(inner) = trimmed
        .strip_prefix("C4<")
        .and_then(|value| value.strip_suffix('>'))
    {
        return normalize_binary_digits(inner).map(Some);
    }

    let Some((_, radix, digits)) = split_verilog_literal(trimmed) else {
        return Ok(None);
    };
    if radix != 'b' {
        return Ok(None);
    }
    normalize_binary_digits(digits).map(Some)
}

fn parse_hex_literal(raw_value: &str) -> Result<String> {
    let trimmed = raw_value.trim();
    let digits = if let Some((_, radix, digits)) = split_verilog_literal(trimmed) {
        if radix != 'h' {
            bail!(
                "unsupported BRAM INIT literal radix '{}' in {trimmed}",
                radix
            );
        }
        digits
    } else {
        trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed)
    };

    let normalized = digits.replace('_', "");
    if normalized.is_empty() {
        return Ok("0".repeat(64));
    }
    if !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("unsupported BRAM INIT payload {trimmed}");
    }
    if normalized.len() > 64 {
        bail!(
            "BRAM INIT payload has {} hex digits, expected at most 64",
            normalized.len()
        );
    }
    Ok(format!("{normalized:0>64}"))
}

fn split_verilog_literal(raw_value: &str) -> Option<(Option<&str>, char, &str)> {
    let (width, rest) = raw_value.split_once('\'')?;
    let mut chars = rest.chars();
    let radix = chars.next()?.to_ascii_lowercase();
    let digits = chars.as_str();
    Some(((!width.is_empty()).then_some(width), radix, digits))
}

fn normalize_binary_digits(raw_value: &str) -> Result<String> {
    let digits = raw_value.replace('_', "");
    if digits.is_empty() {
        return Ok("0".repeat(256));
    }
    if !digits.chars().all(|ch| matches!(ch, '0' | '1')) {
        bail!("unsupported BRAM INIT binary payload {raw_value}");
    }
    if digits.len() > 256 {
        bail!(
            "BRAM INIT binary payload has {} bits, expected at most 256",
            digits.len()
        );
    }
    Ok(format!("{digits:0>256}"))
}

fn hex_words_to_u32s(hex: &str) -> Result<[u32; 8]> {
    let mut words = [0u32; 8];
    for (index, word) in words.iter_mut().enumerate() {
        let end = hex.len() - index * 8;
        let start = end - 8;
        *word = u32::from_str_radix(&hex[start..end], 16)
            .map_err(|error| anyhow!("invalid BRAM INIT hex word {}: {error}", &hex[start..end]))?;
    }
    Ok(words)
}

fn binary_words_to_u32s(binary: &str) -> Result<[u32; 8]> {
    let mut words = [0u32; 8];
    for (index, word) in words.iter_mut().enumerate() {
        let end = binary.len() - index * 32;
        let start = end - 32;
        *word = u32::from_str_radix(&binary[start..end], 2).map_err(|error| {
            anyhow!(
                "invalid BRAM INIT binary word {}: {error}",
                &binary[start..end]
            )
        })?;
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_memory_init_words, build_memory_payloads, hex_words_to_u32s, parse_binary_literal,
        parse_hex_literal, parse_init_payload_words,
    };
    use crate::{
        bitgen::{ProgrammedMemory, ProgrammingImage},
        cil::parse_cil_str,
        resource::{Arch, TileInstance},
    };
    use std::collections::BTreeMap;

    #[test]
    fn parses_cpp_style_bram_init_hex_payloads_as_low_word_first() {
        let words = parse_init_payload_words(
            "256'h0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("parse INIT payload");
        assert_eq!(
            words,
            [
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
            ]
        );
    }

    #[test]
    fn parses_binary_bram_init_payloads_when_present() {
        let binary = parse_binary_literal("256'b1")
            .expect("parse binary")
            .expect("binary");
        assert_eq!(binary.len(), 256);
        let words = parse_init_payload_words("256'b1").expect("parse words");
        assert_eq!(words[0], 1);
        assert!(words[1..].iter().all(|word| *word == 0));
    }

    #[test]
    fn applies_init_words_into_chunk_order() {
        let mut chunk = vec![0u32; 128];
        apply_memory_init_words(
            &mut chunk,
            &[
                (
                    "INIT_00".to_string(),
                    "256'h0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string(),
                ),
                (
                    "INIT_01".to_string(),
                    "256'hfedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                        .to_string(),
                ),
            ],
        )
        .expect("apply init words");

        assert_eq!(
            &chunk[..16],
            &[
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
                0x89ab_cdef,
                0x0123_4567,
                0x7654_3210,
                0xfedc_ba98,
                0x7654_3210,
                0xfedc_ba98,
                0x7654_3210,
                0xfedc_ba98,
                0x7654_3210,
                0xfedc_ba98,
            ]
        );
        assert!(chunk[16..].iter().all(|word| *word == 0));
    }

    #[test]
    fn build_memory_payloads_orders_tiles_by_arch_bit_position() {
        let cil = parse_cil_str(
            r##"
        <device name="mini">
          <bstrcmd_library>
            <parameter name="mem_amount" value="2"/>
          </bstrcmd_library>
        </device>
        "##,
        )
        .expect("parse mini cil");
        let arch = Arch {
            tiles: BTreeMap::from([
                (
                    (0, 0),
                    TileInstance {
                        name: "BRAMR8C3".to_string(),
                        tile_type: "RBRAMD".to_string(),
                        logic_x: 0,
                        logic_y: 0,
                        bit_x: 3,
                        bit_y: 8,
                        phy_x: 0,
                        phy_y: 0,
                    },
                ),
                (
                    (1, 0),
                    TileInstance {
                        name: "BRAMR4C0".to_string(),
                        tile_type: "LBRAMD".to_string(),
                        logic_x: 1,
                        logic_y: 0,
                        bit_x: 0,
                        bit_y: 4,
                        phy_x: 1,
                        phy_y: 0,
                    },
                ),
            ]),
            ..Arch::default()
        };
        let programming = ProgrammingImage {
            memories: vec![
                ProgrammedMemory::new(
                    "BRAMR8C3",
                    vec![(
                        "INIT_00".to_string(),
                        "256'h00000000000000000000000000000000000000000000000000000000deadbeef"
                            .to_string(),
                    )],
                ),
                ProgrammedMemory::new(
                    "BRAMR4C0",
                    vec![(
                        "INIT_00".to_string(),
                        "256'h00000000000000000000000000000000000000000000000000000000cafebabe"
                            .to_string(),
                    )],
                ),
            ],
            ..ProgrammingImage::default()
        };

        let payloads = build_memory_payloads(&cil, &arch, Some(&programming)).expect("payloads");
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0][0], 0xcafe_babe);
        assert_eq!(payloads[1][0], 0xdead_beef);
    }

    #[test]
    fn normalizes_short_hex_payloads_for_unit_tests() {
        let hex = parse_hex_literal("0123456789ABCDEF").expect("hex");
        assert_eq!(hex.len(), 64);
        let words = hex_words_to_u32s(&hex).expect("words");
        assert_eq!(words[0], 0x89ab_cdef);
        assert_eq!(words[1], 0x0123_4567);
        assert!(words[2..].iter().all(|word| *word == 0));
    }
}
