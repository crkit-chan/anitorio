use indexmap::IndexSet;
#[cfg(feature = "kiddo")]
use kiddo::{ImmutableKdTree, SquaredEuclidean};

use crate::hash::IdentityHasherBuilder;
use crate::prelude::*;

pub type ColorPalettePixel = [u8; 3];
pub type ColorPalettePixelSlice = indexmap::set::Slice<ColorPalettePixel>;

pub struct ColorPalette {
    colors: IndexSet<ColorPalettePixel, IdentityHasherBuilder>,
    #[cfg(feature = "kiddo")]
    tree:   ImmutableKdTree<u8, 3>,
}
impl ColorPalette {
    #[allow(clippy::unnecessary_wraps, reason = "Result<> used for 'kiddo'")]
    pub fn new(mut colors: Box<[ColorPalettePixel]>) -> Result<Self, JsError> {
        colors.sort_unstable(); // Allows better compression

        Ok(Self {
            #[cfg(feature = "kiddo")]
            tree: ImmutableKdTree::new_from_slice(&colors)?,
            colors: colors.into_iter().collect(),
        })
    }

    #[inline]
    #[must_use]
    pub fn get_colors(&self) -> &ColorPalettePixelSlice {
        self.colors.as_slice()
    }

    /// Finds the exact index of a pixel. This uses a cached ``HashMap`` and
    /// will error if the pixel colour is not inside the ``HashMap``.
    #[inline]
    #[must_use]
    pub fn exact_index(&self, target: ColorPalettePixel) -> u32 {
        #[allow(clippy::unwrap_used, reason = "Convenience function, should panic!")]
        self.colors.get_index_of(&target).unwrap() as u32
    }

    #[inline]
    #[must_use]
    pub fn nearest_index(&self, target: ColorPalettePixel) -> u32 {
        #[cfg(feature = "kiddo")]
        {
            self.tree.query(&target).nearest_one::<SquaredEuclidean<f32>>().execute().item
        }

        #[cfg(not(feature = "kiddo"))]
        {
            let mut best_dist = i32::MAX;
            let mut best_idx = 0_u32;

            for (i, color) in self.colors.iter().enumerate() {
                let dr = i32::from(target[0]) - i32::from(color[0]);
                let dg = i32::from(target[1]) - i32::from(color[1]);
                let db = i32::from(target[2]) - i32::from(color[2]);

                let dist = dr * dr + dg * dg + db * db;

                if dist < best_dist {
                    best_idx = i as u32;
                    if dist == 0 {
                        break;
                    }
                    best_dist = dist;
                }
            }

            best_idx
        }
    }
}
