/// An identity hasher for values whose representation fits within a [`u64`].
///
/// Performs no actual hashing. Not suitable for untrusted or arbitrary input.
pub type IdentityHasherBuilder = std::hash::BuildHasherDefault<IdentityHasher>;

pub struct IdentityHasher(u64);
impl std::hash::Hasher for IdentityHasher {
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
impl const Default for IdentityHasher {
    fn default() -> Self {
        Self(Default::default())
    }
}
