use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use pumpkin_data::BlockStateId;
use pumpkin_util::encompassing_bits;
use pumpkin_world::chunk::palette::{BiomePalette, BlockPalette};

/// Palette sizes seen in real sections: a few blocks, a cave, a village, a build.
const PALETTE_LENS: [usize; 5] = [3, 12, 40, 200, 300];

/// Random indices below `len`, packed as Anvil and the network do.
fn packed(len: usize, min_bits: u8, seed: &mut u64) -> Vec<i64> {
    let bits = encompassing_bits(len).max(min_bits);
    let per_word = 64 / usize::from(bits);
    (0..4096usize.div_ceil(per_word))
        .map(|_| {
            (0..per_word).fold(0u64, |word, i| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                word | (*seed % len as u64) << (i * usize::from(bits))
            }) as i64
        })
        .collect()
}

fn block_states(len: usize) -> Vec<BlockStateId> {
    (1..=len as u16).map(BlockStateId::new_or_air).collect()
}

fn bench_palette(c: &mut Criterion) {
    let mut seed = 0x2545_F491_4F6C_DD1D;
    let mut encode = c.benchmark_group("block palette to network");
    for len in PALETTE_LENS {
        let palette = block_states(len);
        let section =
            BlockPalette::from_palette_and_packed_data(&palette, &packed(len, 4, &mut seed), 4);
        encode.bench_with_input(BenchmarkId::from_parameter(len), &section, |b, section| {
            b.iter(|| black_box(section.convert_network()));
        });
    }
    encode.finish();

    let mut decode = c.benchmark_group("block palette from anvil");
    for len in PALETTE_LENS {
        let palette = block_states(len);
        let data = packed(len, 4, &mut seed);
        decode.bench_with_input(BenchmarkId::from_parameter(len), &data, |b, data| {
            b.iter(|| {
                black_box(BlockPalette::from_palette_and_packed_data(
                    &palette, data, 4,
                ))
            });
        });
    }
    decode.finish();

    let biomes: Vec<u8> = (0..6).collect();
    let section = BiomePalette::from_palette_and_packed_data(&biomes, &packed(6, 1, &mut seed), 1);
    c.bench_function("biome palette to network", |b| {
        b.iter(|| black_box(section.convert_network()));
    });
}

criterion_group!(benches, bench_palette);
criterion_main!(benches);
