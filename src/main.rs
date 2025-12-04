#![feature(portable_simd)]
#![feature(likely_unlikely)]

mod hashmap;
mod xor;

use core::str;
use std::hash::{Hash, Hasher};
use std::simd::cmp::SimdPartialEq;
use std::simd::{Simd, u8x16};

use memmap2::Mmap;

use crate::hashmap::{KeyHashPair, SimpleHashMap};
use crate::xor::XorHash;

const DEBUG: bool = true;

type HashMap<'a> = SimpleHashMap<StationName<'a>, StationSummary, XorHash>;

fn main() {
    const { assert!(cfg!(target_endian = "little")) };

    let measurements_file = std::env::args()
        .nth(1)
        .expect("Missing measurements file argument");
    let file = std::fs::File::open(measurements_file).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };
    let file_bytes: &[u8] = mmap.as_ref();

    // sometimes we read 128 bytes ahead, without checking if we reached EOF.
    // to avoid reading past EOF, we find the last newline before the last 128 bytes,
    // and split the file there. The main loop will process the first part without bounds checks,
    // and the second part (the "remainder") with bounds checks.
    let remainder_idx = {
        let idx = file_bytes.len() - 128;
        idx - file_bytes[..idx]
            .iter()
            .rev()
            .position(|&b| b == b'\n')
            .unwrap()
    };
    let (file_bytes, mut file_bytes_remainder) = file_bytes.split_at(remainder_idx);

    let workers_num = std::thread::available_parallelism().unwrap().get();
    let mut measurements = std::thread::scope(|scope| {
        // Split the file into chunks for each worker
        let file_bytes = split_bytes_aligned(file_bytes, workers_num);

        // Spawn worker threads
        let workers = file_bytes
            .into_iter()
            .map(|file_bytes| scope.spawn(move || parse_file_bytes(file_bytes)))
            .collect::<Vec<_>>();

        // Merge results
        let measurements = workers.into_iter().map(|w| w.join().unwrap()).reduce(
            |mut measurements, worker_measurements| {
                for (station_name, summary) in worker_measurements.iter() {
                    let global_summary = measurements.get_or_default(*station_name);
                    global_summary.min = global_summary.min.min(summary.min);
                    global_summary.max = global_summary.max.max(summary.max);
                    global_summary.sum += summary.sum;
                    global_summary.count += summary.count;
                }
                measurements
            },
        );
        measurements.unwrap()
    });

    // process remainder (trivially, no optimizations)
    while !file_bytes_remainder.is_empty() {
        let newline_pos = file_bytes_remainder
            .iter()
            .position(|&b| b == b'\n')
            .unwrap();
        let line = &file_bytes_remainder[..newline_pos];
        file_bytes_remainder = &file_bytes_remainder[newline_pos + 1..]; // skip newline

        let semicolon_pos = line.iter().position(|&b| b == b';').unwrap();
        let name_bytes = &line[..semicolon_pos];
        let measurement_bytes = &line[semicolon_pos + 1..]; // skip semicolon
        let station_name = StationName::new(name_bytes);
        let measurement = std::str::from_utf8(measurement_bytes)
            .unwrap()
            .parse::<f64>()
            .unwrap();

        measurements
            .get_or_default(station_name)
            .update((measurement * 10.0) as i16);
    }

    // output
    // format: {Abha=-23.0/18.0/59.2, Abidjan=-16.2/26.0/67.3, Abéché=-10.0/29.4/69.0, Accra=-10.1/26.4/66.4, Addis Ababa=-23.7/16.0/67.0, Adelaide=-27.8/17.3/58.5, ...}
    let mut measurements_sorted = measurements
        .iter()
        .map(|(name, m)| (name.to_str(), m))
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

#[inline(never)]
fn parse_file_bytes<'a>(file_bytes: &'a [u8]) -> HashMap<'a> {
    // To utilize the CPU pipeline better, we maintain a batch of cursors into the file,
    // and process them in parallel (in the same thread).
    // Every variable that you expected to be u32, is now [u32; BATCH].
    const BATCH: usize = 4;
    fn batch<T>(f: impl FnMut(usize) -> T) -> [T; BATCH] {
        std::array::from_fn(f)
    }

    // Split the file into BATCH parts
    let (mut file_ptr, file_end) = {
        let splits = split_bytes_aligned(file_bytes, BATCH);
        let file_ptr = batch(|bi| splits[bi].as_ptr());
        let file_end = batch(|bi| unsafe { splits[bi].as_ptr().add(splits[bi].len()) });
        (file_ptr, file_end)
    };

    // Main loop
    let mut measurements = HashMap::new(1000, 128.0);
    while std::hint::likely((0..BATCH).all(|bi| file_ptr[bi] < file_end[bi])) {
        // format: <string: station name>;<double: measurement>

        // Read the name of the station
        let first_word = batch(|bi| unsafe { file_ptr[bi].cast::<u128>().read_unaligned() });
        let station_name = batch(|bi| unsafe {
            StationName::parse_and_hash(&mut file_ptr[bi], first_word[bi], measurements.hasher())
        });

        // Read the temperature measurement
        let measurement = batch(|bi| unsafe { parse_temperature(&mut file_ptr[bi]) });

        // Update per-station summary
        batch(|bi| {
            measurements
                .get_or_default(station_name[bi])
                .update(measurement[bi]);
        });
    }

    // Process remaining bytes in each batch cursor
    batch(|bi| {
        // same implementation as the main loop, but for a single cursor instead of BATCH

        let (mut file_ptr, file_end) = (file_ptr[bi], file_end[bi]);
        while std::hint::likely(file_ptr < file_end) {
            let first_word = unsafe { file_ptr.cast::<u128>().read_unaligned() };
            let station_name = unsafe {
                StationName::parse_and_hash(&mut file_ptr, first_word, measurements.hasher())
            };
            let measurement = unsafe { parse_temperature(&mut file_ptr) };
            measurements
                .get_or_default(station_name)
                .update(measurement);
        }
    });

    measurements
}

#[derive(Clone, Copy)]
struct StationName<'a> {
    // The first 16 bytes of the name, stored as u128 for fast comparisons and hashing
    // If the name is shorter than 16 bytes, the upper bytes are zeroed
    prefix: u128,
    // Pointer to the remainder of the name (after the first 16 bytes).
    // Its valid to dereference the 16 bytes before this pointer, as they are part of the name.
    // We store the pointer to the remainder instead of the beginning of the name as most of the times
    // we want to access only the remainder (for equality checks and hashing).
    remainder_ptr: *const u8,
    // Length of the remainder (can be negative if the name is shorter than 16 bytes)
    remainder_len: isize,
    ph: std::marker::PhantomData<&'a [u8]>,
}
impl<'a> StationName<'a> {
    pub fn new(name_bytes: &'a [u8]) -> Self {
        let mut prefix_bytes = [0_u8; 16];
        let prefix_len = name_bytes.len().min(16);
        prefix_bytes[..prefix_len].copy_from_slice(&name_bytes[..prefix_len]);
        let prefix = u128::from_ne_bytes(prefix_bytes);
        Self::new_with_prefix(prefix, name_bytes)
    }

    pub fn new_with_prefix(prefix: u128, full_name: &'a [u8]) -> Self {
        Self {
            prefix,
            remainder_ptr: unsafe { full_name.as_ptr().add(16) },
            remainder_len: full_name.len().cast_signed() - 16,
            ph: std::marker::PhantomData,
        }
    }

    unsafe fn parse_and_hash(
        file_ptr: &mut *const u8,
        first_word: u128,
        hash: &impl std::hash::BuildHasher,
    ) -> KeyHashPair<Self> {
        let mut name_prefix = first_word;
        let name_length;
        let mut hash = hash.build_hasher();

        let semicolon_pos = u8x16::from_array(name_prefix.to_ne_bytes())
            .simd_eq(u8x16::splat(b';'))
            .to_bitmask()
            .trailing_zeros() as usize;
        if semicolon_pos <= 16 {
            // fast path, semicolon is in the first 16 bytes

            name_length = semicolon_pos;
            // zero the upper bytes of name_prefix
            name_prefix &= (1_u128.wrapping_shl((name_length * 8) as u32)) - 1;
            hash.write_u128(name_prefix);
        } else {
            // slow path, semicolon is after the first 16 bytes

            hash.write_u128(name_prefix);
            let mut offset = 16;
            loop {
                const STEP_WORDS: usize = 1;
                const STEP_BYTES: usize = STEP_WORDS * 8;
                let words = unsafe {
                    file_ptr
                        .add(offset)
                        .cast::<[u64; STEP_WORDS]>()
                        .read_unaligned()
                };
                let words_bytes =
                    unsafe { std::mem::transmute::<[u64; STEP_WORDS], [u8; STEP_BYTES]>(words) };
                let value_pos = Simd::<u8, _>::from_array(words_bytes)
                    .simd_eq(Simd::splat(b';'))
                    .to_bitmask()
                    .trailing_zeros() as usize;
                if value_pos < STEP_BYTES {
                    name_length = offset + value_pos;
                    for word in words.iter().take(value_pos / 8) {
                        hash.write_u64(*word);
                    }
                    hash.write_u64(
                        words[value_pos / 8]
                            & ((1_u64.wrapping_shl(((value_pos % 8) * 8) as u32)) - 1),
                    );
                    break;
                }
                offset += STEP_BYTES;
                for word in words {
                    hash.write_u64(word);
                }
            }
        };

        let full_name = unsafe { std::slice::from_raw_parts(*file_ptr, name_length) };
        *file_ptr = unsafe { file_ptr.add(name_length + 1) }; // skip semicolon

        let hash = hash.finish();
        let name = StationName::new_with_prefix(name_prefix, full_name);
        unsafe { KeyHashPair::new_unchecked(name, hash) }
    }

    fn remainder(&self) -> &[u8] {
        debug_assert!(self.remainder_len >= 0);
        unsafe {
            std::slice::from_raw_parts(self.remainder_ptr, self.remainder_len.cast_unsigned())
        }
    }

    #[inline(never)]
    #[cold]
    fn to_str(&self) -> &str {
        let len = (self.remainder_len + 16).cast_unsigned();
        let full_name_ptr = unsafe { self.remainder_ptr.offset(-16) };
        let full_name = unsafe { std::slice::from_raw_parts(full_name_ptr, len) };
        str::from_utf8(full_name).unwrap()
    }
}
impl Hash for StationName<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.prefix.hash(state);
        if self.remainder_len > 0 {
            state.write(self.remainder());
        }
    }
}
impl PartialEq for StationName<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.prefix != other.prefix {
            return false;
        }
        if self.remainder_len <= 0 {
            debug_assert_eq!(self.remainder_len, other.remainder_len);
            return true; // prefixes are equal, and no remainders
        }
        if self.remainder_len != other.remainder_len {
            return false;
        }
        self.remainder() == other.remainder()
    }
}
impl Eq for StationName<'_> {}
unsafe impl<'a> Send for StationName<'a> {}
unsafe impl<'a> Sync for StationName<'a> {}

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
unsafe fn parse_temperature(file_ptr: &mut *const u8) -> i16 {
    let newline_pos = Simd::from_array(unsafe { (*file_ptr).cast::<[u8; 8]>().read() })
        .simd_eq(Simd::splat(b'\n'))
        .to_bitmask()
        .trailing_zeros() as usize;
    unsafe { std::hint::assert_unchecked(newline_pos < 8) };
    let s = unsafe { std::slice::from_raw_parts(*file_ptr, newline_pos) };

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

    *file_ptr = unsafe { file_ptr.add(newline_pos + 1) }; // skip newline

    value
}

#[inline(never)]
fn split_bytes_aligned(bytes: &[u8], splits_num: usize) -> Vec<&[u8]> {
    assert!(splits_num >= 1);
    let mut split_indices = Vec::with_capacity(splits_num - 1);
    for i in 1..splits_num {
        let idx = (i as f64 * bytes.len() as f64 / splits_num as f64) as usize;
        let aligned_idx = idx + bytes[idx..].iter().position(|&b| b == b'\n').unwrap() + 1;
        split_indices.push(aligned_idx);
    }
    (0..splits_num)
        .map(|i| {
            let start = if i == 0 { 0 } else { split_indices[i - 1] };
            let end = split_indices.get(i).copied().unwrap_or(bytes.len());
            &bytes[start..end]
        })
        .collect()
}
