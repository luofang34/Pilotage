//! A deterministic procedural ground texture: fields of varied colour on
//! a grid of roads. It needs no network, so tests and offline runs use it.

use std::collections::HashMap;
use std::sync::Arc;

use super::{TILE_SIZE, TileImage, TileKey, TileSource};

/// The field size, metres of web-mercator distance at the equator scale.
const FIELD_M: f64 = 60.0;
/// The road width, metres.
const ROAD_M: f64 = 4.0;
/// Web-mercator metres across the whole world.
const WORLD_M: f64 = 40_075_016.686;
/// Tiles kept in memory.
const CAPACITY: usize = 64;

/// The colour of the procedural ground at a global web-mercator position,
/// in metres from the north-west corner of the world.
#[must_use]
pub fn ground_rgb(x_m: f64, y_m: f64, seed: u64) -> [u8; 3] {
    let column = (x_m / FIELD_M).floor();
    let row = (y_m / FIELD_M).floor();
    let along_x = x_m - column * FIELD_M;
    let along_y = y_m - row * FIELD_M;
    if along_x < ROAD_M || along_y < ROAD_M {
        return [118, 116, 110];
    }
    let hash = mix(seed ^ mix((column as i64) as u64 ^ mix((row as i64) as u64)));
    let palette = [
        [86, 122, 58],
        [124, 140, 74],
        [152, 138, 92],
        [70, 98, 52],
        [168, 160, 112],
    ];
    let base = palette[(hash % palette.len() as u64) as usize];
    // Rows of crops give each field a direction and some texture.
    let stripe = if hash & 0x100 == 0 { along_x } else { along_y };
    let shade = if (stripe / 3.0).floor() as i64 % 2 == 0 {
        0
    } else {
        10
    };
    base.map(|channel: u8| channel.saturating_sub(shade))
}

/// The SplitMix64 finalizer.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Tiles of the procedural ground texture.
#[derive(Debug)]
pub struct ProceduralTiles {
    seed: u64,
    cache: HashMap<TileKey, Arc<TileImage>>,
}

impl ProceduralTiles {
    /// A source of the texture for `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            cache: HashMap::new(),
        }
    }

    fn render(&self, key: TileKey) -> Option<TileImage> {
        let tiles = f64::from(1u32 << u32::from(key.z.min(30)));
        let metres_per_pixel = WORLD_M / (tiles * f64::from(TILE_SIZE));
        let mut rgb = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 3) as usize);
        for py in 0..TILE_SIZE {
            for px in 0..TILE_SIZE {
                let x = (f64::from(key.x) * f64::from(TILE_SIZE) + f64::from(px) + 0.5)
                    * metres_per_pixel;
                let y = (f64::from(key.y) * f64::from(TILE_SIZE) + f64::from(py) + 0.5)
                    * metres_per_pixel;
                rgb.extend_from_slice(&ground_rgb(x, y, self.seed));
            }
        }
        TileImage::from_rgb(rgb)
    }
}

impl TileSource for ProceduralTiles {
    fn tile(&mut self, key: TileKey) -> Option<Arc<TileImage>> {
        if let Some(tile) = self.cache.get(&key) {
            return Some(Arc::clone(tile));
        }
        let tile = Arc::new(self.render(key)?);
        if self.cache.len() >= CAPACITY {
            self.cache.clear();
        }
        self.cache.insert(key, Arc::clone(&tile));
        Some(tile)
    }
}
