#![feature(portable_simd)]
#![feature(likely_unlikely)]

mod hashmap;
mod xor;

use core::str;
use std::hash::{Hash, Hasher};
use std::simd::cmp::SimdPartialEq;
use std::simd::u8x16;

use memmap2::Mmap;

use crate::hashmap::{KeyHashPair, SimpleHashMap};
use crate::xor::XorHash;

const DEBUG: bool = true;

type HashMap<'a> = SimpleHashMap<StationName<'a>, StationSummary, XorHash>;

fn main() {
    assert!(cfg!(target_endian = "little"));

    let measurements_file = std::env::args()
        .skip(1)
        .next()
        .expect("Missing measurements file argument");
    let file = std::fs::File::open(measurements_file).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };
    let file_bytes: &[u8] = mmap.as_ref();

    const BATCH: usize = 4;
    fn batch<T>(f: impl FnMut(usize) -> T) -> [T; BATCH] {
        std::array::from_fn(f)
    }
    let (mut file_ptr, file_end) = {
        let idx = batch(|bi| {
            let idx = (bi as f64 * file_bytes.len() as f64 / BATCH as f64) as usize;
            let aligned_idx = idx + file_bytes[idx..].iter().position(|&b| b == b'\n').unwrap();
            aligned_idx
        });
        let file_ptr = batch(|bi| unsafe { file_bytes.as_ptr().add(idx[bi]) });
        let file_end = batch(|bi| unsafe {
            file_bytes
                .as_ptr()
                .add(*idx.get(bi + 1).unwrap_or(&file_bytes.len()))
        });
        (file_ptr, file_end)
    };

    let mut measurements = HashMap::new(1000, 128.0);
    while std::hint::likely((0..BATCH).all(|bi| file_ptr[bi] < file_end[bi])) {
        let newline_pos = batch(|bi| find_simd_multi(file_ptr[bi], b'\n'));

        // format: <string: station name>;<double: measurement>
        let semicolon_pos = batch(|bi| find_simd_multi(file_ptr[bi], b';'));
        let station_name = batch(|bi| unsafe {
            StationName::parse_and_hash(file_ptr[bi], semicolon_pos[bi], measurements.hasher())
        });

        let measurement = batch(|bi| unsafe {
            parse_temperature(std::slice::from_raw_parts(
                file_ptr[bi].add(semicolon_pos[bi] + 1),
                newline_pos[bi] - semicolon_pos[bi] - 1,
            ))
        });

        file_ptr = batch(|bi| unsafe { file_ptr[bi].add(newline_pos[bi] + 1) }); // skip newline

        batch(|bi| {
            measurements
                .get_or_default(station_name[bi])
                .update(measurement[bi]);
        });
    }
    batch(|bi| {
        let mut file_ptr = file_ptr[bi];
        let file_end = file_end[bi];
        while std::hint::likely(file_ptr < file_end) {
            let newline_pos = find_simd_multi(file_ptr, b'\n');

            // format: <string: station name>;<double: measurement>
            let semicolon_pos = find_simd_multi(file_ptr, b';');
            let station_name = unsafe {
                StationName::parse_and_hash(file_ptr, semicolon_pos, measurements.hasher())
            };

            let measurement = unsafe {
                parse_temperature(std::slice::from_raw_parts(
                    file_ptr.add(semicolon_pos + 1),
                    newline_pos - semicolon_pos - 1,
                ))
            };

            file_ptr = unsafe { file_ptr.add(newline_pos + 1) }; // skip newline

            measurements
                .get_or_default(station_name)
                .update(measurement);
        }
    });

    // output
    // format: {Abha=-23.0/18.0/59.2, Abidjan=-16.2/26.0/67.3, Abéché=-10.0/29.4/69.0, Accra=-10.1/26.4/66.4, Addis Ababa=-23.7/16.0/67.0, Adelaide=-27.8/17.3/58.5, ...}
    let mut measurements_sorted = measurements
        .iter()
        .map(|(name, m)| (name.to_string(), m))
        .collect::<Vec<_>>();
    measurements_sorted.sort_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));

    let mut output = String::from("{");
    for (station_name, summary) in &measurements_sorted {
        let avg = summary.sum as f64 / 10.0 / summary.count as f64;
        output.push_str(&format!(
            "{station_name}={:.1}/{:.1}/{:.1}, ",
            summary.min as f32 / 10.0,
            summary.max as f32 / 10.0,
            avg
        ));
    }
    output.pop();
    output.pop();
    output.push('}');
    println!("{}", output);

    if DEBUG {
        println!(
            "Fallback size: {}/{}",
            measurements.fallback_size(),
            measurements_sorted.len()
        );
    }
}

// the name contains ';' at the end
#[derive(Clone, Copy)]
struct StationName<'a> {
    prefix: u128,
    remainder_len: isize,
    full_name: *const u8,
    ph: std::marker::PhantomData<&'a [u8]>,
}
impl<'a> StationName<'a> {
    pub fn new(prefix: u128, full_name: &'a [u8]) -> Self {
        Self {
            prefix,
            full_name: full_name.as_ptr(),
            remainder_len: full_name.len().cast_signed() - 16,
            ph: std::marker::PhantomData,
        }
    }

    unsafe fn parse_and_hash(
        file_ptr: *const u8,
        semicolon_pos: usize,
        hash: &impl std::hash::BuildHasher,
    ) -> KeyHashPair<Self> {
        let mut name_prefix = unsafe { file_ptr.cast::<u128>().read_unaligned() };
        let name_length = semicolon_pos + 1; // keep the semicolon
        let full_name = unsafe { std::slice::from_raw_parts(file_ptr, name_length) };

        let mut hash = hash.build_hasher();
        if name_length <= 16 {
            // zero the upper bytes of name_prefix
            name_prefix &= (1_u128.wrapping_shl((name_length * 8) as u32)) - 1;
            hash.write_u128(name_prefix);
        } else {
            hash.write_u128(name_prefix);
            hash.write(&full_name[16..]);
        }
        let hash = hash.finish();
        let name = StationName::new(name_prefix, full_name);
        unsafe { KeyHashPair::new_unchecked(name, hash) }
    }

    #[inline(never)]
    #[cold]
    fn full_name(&self) -> &[u8] {
        let len = (self.remainder_len + 16).cast_unsigned();
        let len_without_semicolon = len - 1;
        unsafe { std::slice::from_raw_parts(self.full_name, len_without_semicolon) }
    }

    fn remainder(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self.full_name.add(16), self.remainder_len.cast_unsigned())
        }
    }
}
impl Hash for StationName<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.prefix.hash(state);
        if self.remainder_len > 0 {
            state.write(&self.remainder());
        }
    }
}
impl PartialEq for StationName<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.prefix != other.prefix {
            return false;
        }
        if self.remainder_len <= 0 {
            return true;
        }
        if self.remainder_len != other.remainder_len {
            return false;
        }
        self.remainder() == other.remainder()
    }
}
impl Eq for StationName<'_> {}
impl ToString for StationName<'_> {
    fn to_string(&self) -> String {
        str::from_utf8(self.full_name()).unwrap().to_string()
    }
}

struct StationSummary {
    min: i16,
    max: i16,
    sum: i64,
    count: u32,
}
impl Default for StationSummary {
    fn default() -> Self {
        Self {
            min: i16::MAX,
            max: i16::MIN,
            sum: 0,
            count: 0,
        }
    }
}
impl StationSummary {
    fn update(&mut self, measurement: i16) {
        if std::hint::unlikely(measurement < self.min) {
            self.min = measurement;
        }
        if std::hint::unlikely(measurement > self.max) {
            self.max = measurement;
        }
        self.sum += measurement as i64;
        self.count += 1;
    }
}

/// # Safety
///
/// It must be OK to dereference `s.as_ptr().offset(-1)``, doesn't matter what this address contains
#[inline(always)]
unsafe fn parse_temperature(s: &[u8]) -> i16 {
    #[inline(always)]
    unsafe fn parse_temperature_impl(s: &[u8]) -> i16 {
        let len = s.len() as isize;
        let p = s.as_ptr();
        unsafe {
            let frac = *p.offset(len - 1) - b'0';
            let d0 = *p.offset(len - 3) - b'0';
            let d1 = (*p.offset(len - 4)).wrapping_sub(b'0');
            let positive = *p != b'-';

            let d1_valid = len >= 5 - (positive as isize);

            let mut value =
                /* digit -1 */  (frac as i16)
                /* digit 0 */ + (d0 as i16 * 10)
                /* digit 1 */ + ((d1 * (d1_valid as u8)) as i16 * 100);
            value *= ((positive as i16) << 1) - 1;
            value
        }
    }

    let value = unsafe { parse_temperature_impl(s) };

    #[cfg(debug_assertions)]
    {
        let s = str::from_utf8(s).unwrap();
        let expected_value = s.parse::<f64>().unwrap();
        debug_assert_eq!(
            value,
            (expected_value * 10.0) as i16,
            "parsed value does not match standard library parsing for str '{s}'"
        );
    }
    value
}

#[inline(always)]
fn find_simd_single(ptr: *const u8, val: u8) -> usize {
    let ptr = ptr.cast::<[u8; 16]>();
    let word: [u8; 16] = unsafe { ptr.read() };
    u8x16::from_array(word)
        .simd_eq(u8x16::splat(val))
        .to_bitmask()
        .trailing_zeros() as usize
}

#[inline(always)]
fn find_simd_multi(ptr: *const u8, val: u8) -> usize {
    let value_pos = find_simd_single(ptr, val);
    if value_pos < 16 {
        return value_pos;
    }
    for i in 1.. {
        let value_pos = find_simd_single(unsafe { ptr.add(i * 16) }, val);
        if value_pos < 16 {
            return i * 16 + value_pos;
        }
    }
    unreachable!()
}
