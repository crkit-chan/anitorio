pub struct U32Hasher(u64);

impl U32Hasher {
    #[allow(dead_code, reason = "bug")]
    pub type BuildDefault = std::hash::BuildHasherDefault<Self>;
}
impl std::hash::Hasher for U32Hasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = 0_u64;
        for &byte in bytes {
            hash = (hash << 8) | u64::from(byte);
        }
        self.0 = hash;
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = u64::from(i);
    }
}
impl const Default for U32Hasher {
    fn default() -> Self {
        Self(Default::default())
    }
}
