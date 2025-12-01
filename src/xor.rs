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
    fn write_u64(&mut self, i: u64) {
        self.0 ^= i as u64;
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        while let Some(chunk) = chunks.next() {
            let chunk: [u8; 8] = unsafe { chunk.try_into().unwrap_unchecked() };
            let chunk = u64::from_ne_bytes(chunk);
            self.write_u64(chunk);
        }

        let mut remainder = [0_u8; 8];
        remainder[..chunks.remainder().len()].copy_from_slice(chunks.remainder());
        let remainder = u64::from_ne_bytes(remainder);
        self.write_u64(remainder);
    }

    fn finish(&self) -> u64 {
        let hash = self.0;
        hash ^ (hash >> 33) ^ (hash >> 15)
    }

    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64);
    }
    fn write_u16(&mut self, i: u16) {
        self.write_u64(i as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_u128(&mut self, i: u128) {
        self.write_u64(unsafe { (&i as *const u128).cast::<u64>().add(0).read() });
        self.write_u64(unsafe { (&i as *const u128).cast::<u64>().add(1).read() });
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}
