use std::hash::{BuildHasher, Hasher};

#[derive(Default)]
pub(crate) struct XorHash;
impl BuildHasher for XorHash {
    type Hasher = XorHasher;

    fn build_hasher(&self) -> Self::Hasher {
        XorHasher(0xd13c02cbc35e3d1d)
    }
}

pub(crate) struct XorHasher(u64);
impl Hasher for XorHasher {
    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.0 ^= i;
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let (mut bytes, mut len) = (bytes.as_ptr(), bytes.len());
        while len > 8 {
            self.write_u64(unsafe { bytes.cast::<u64>().read_unaligned() });
            bytes = unsafe { bytes.add(8) };
            len -= 8;
        }
        for i in 0..len {
            self.0 ^= unsafe { *bytes.add(i) as u64 } << (i * 8);
        }
    }

    #[inline(always)]
    fn finish(&self) -> u64 {
        let hash = self.0;
        hash ^ (hash >> 33) ^ (hash >> 15)
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64);
    }
    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.write_u64(i as u64);
    }
    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    #[inline(always)]
    fn write_u128(&mut self, i: u128) {
        self.write_u64(unsafe { (&i as *const u128).cast::<u64>().add(0).read() });
        self.write_u64(unsafe { (&i as *const u128).cast::<u64>().add(1).read() });
    }
    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}
