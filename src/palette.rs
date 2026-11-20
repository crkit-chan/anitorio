#[derive(Clone)]
pub struct ColorPalette {
    pub colors: Box<[[u8; 3]]>,
}
impl ColorPalette {
    pub fn new(colors: Vec<[u8; 3]>) -> Self {
        assert!(u8::try_from(colors.len().saturating_sub(1)).is_ok());
        Self { colors: colors.into() }
    }

    #[inline]
    pub fn nearest_index(&self, target: [u8; 3]) -> u8 {
        let mut best_dist = u32::MAX;
        let mut best_idx = 0;

        for (i, color) in self.colors.iter().enumerate() {
            let dr = i32::from(target[0]) - i32::from(color[0]);
            let dg = i32::from(target[1]) - i32::from(color[1]);
            let db = i32::from(target[2]) - i32::from(color[2]);

            let dist = (dr * dr + dg * dg + db * db) as u32;

            if dist < best_dist {
                best_dist = dist;
                best_idx = i as u8;
            }
        }

        best_idx
    }
}
