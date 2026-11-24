use crate::prelude::*;
#[cfg(feature = "kiddo")]
use kiddo::{ImmutableKdTree, SquaredEuclidean};

pub type ColorPalettePixel = [u8; 3];
pub struct ColorPalette {
    pub colors: Box<[ColorPalettePixel]>,

    #[cfg(feature = "kiddo")]
    tree: ImmutableKdTree<u8, 3>,
}
impl ColorPalette {
    #[allow(
        clippy::allow_attributes,
        clippy::unnecessary_wraps,
        reason = "Result<> used for 'kiddo'"
    )]
    pub fn new(colors: Box<[ColorPalettePixel]>) -> Result<Self, JsError> {
        Ok(Self {
            #[cfg(feature = "kiddo")]
            tree: ImmutableKdTree::new_from_slice(&colors)?,
            colors,
        })
    }

    #[inline]
    pub fn nearest_index(&self, target: ColorPalettePixel) -> u32 {
        #[cfg(feature = "kiddo")]
        {
            self.tree.query(&target).nearest_one::<SquaredEuclidean<f32>>().execute().item
        }

        #[cfg(not(feature = "kiddo"))]
        {
            let mut best_dist = i32::MAX;
            let mut best_idx = 0u32;

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
