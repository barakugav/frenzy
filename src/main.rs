#![feature(portable_simd)]

mod hashmap;
mod xor;

use core::str;
use std::hash::Hash;
use std::path::Path;
use std::simd::cmp::SimdPartialEq;
use std::simd::u8x16;

use memmap2::Mmap;

use crate::hashmap::SimpleHashMap;
use crate::xor::XorHash;

const DEBUG: bool = true;

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

fn main() {
    assert!(cfg!(target_endian = "little"));

    let measurements_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("1brc")
        .join("measurements.txt");
    let file = std::fs::File::open(measurements_file).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };
    let file_bytes: &[u8] = mmap.as_ref();
    let (mut file_bytes, mut file_len) = (file_bytes.as_ptr(), file_bytes.len());

    let mut measurements = SimpleHashMap::<StationName, StationSummary, XorHash>::new(1000, 128.0);
    while file_len > 0 {
        let newline_pos = find_simd(file_bytes, b'\n');
        let line = unsafe { std::slice::from_raw_parts(file_bytes, newline_pos) };
        file_bytes = unsafe { file_bytes.add(newline_pos + 1) }; // skip newline
        file_len -= newline_pos + 1;

        // format: <string: station name>;<double: measurement>
        let semicolon_pos = find_simd(line.as_ptr(), b';');
        let station_name = {
            let mut name_prefix = unsafe { line.as_ptr().cast::<u128>().read_unaligned() };
            let name_length = semicolon_pos + 1; // keep the semicolon
            let full_name = &line[..name_length];
            // zero the upper bytes of name_prefix
            name_prefix &= (1_u128 << (name_length.min(16) * 8)) - 1;
            StationName::new(name_prefix, full_name)
        };

        let measurement: i16 = unsafe { parse_temperature(&line[semicolon_pos + 1..]) };

        let summary = measurements.get_or_default(station_name);
        if measurement < summary.min {
            summary.min = measurement;
        }
        if measurement > summary.max {
            summary.max = measurement;
        }
        summary.sum += measurement as i64;
        summary.count += 1;
    }

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
struct StationName<'a> {
    prefix: u128,
    len: usize,
    full_name: *const u8,
    ph: std::marker::PhantomData<&'a [u8]>,
}
impl<'a> StationName<'a> {
    pub fn new(prefix: u128, full_name: &'a [u8]) -> Self {
        Self {
            prefix,
            full_name: full_name.as_ptr(),
            len: full_name.len(),
            ph: std::marker::PhantomData,
        }
    }

    fn full_name(&self) -> &[u8] {
        let len_without_semicolon = self.len - 1;
        unsafe { std::slice::from_raw_parts(self.full_name, len_without_semicolon) }
    }

    fn remainder(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.full_name.add(16), self.len - 16) }
    }
}
impl Hash for StationName<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.prefix.hash(state);
        if self.len > 16 {
            self.remainder().hash(state);
        }
    }
}
impl PartialEq for StationName<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.prefix != other.prefix {
            return false;
        }
        if self.len <= 16 {
            return true;
        }
        if self.len != other.len {
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

fn find_simd(ptr: *const u8, val: u8) -> usize {
    let ptr = ptr.cast::<[u8; 16]>();
    let word: [u8; 16] = unsafe { ptr.read() };
    let word = std::simd::u8x16::from_array(word);
    let newline_pos = word
        .simd_eq(u8x16::splat(val))
        .to_bitmask()
        .trailing_zeros() as usize;
    if newline_pos < 16 {
        return newline_pos;
    }
    for i in 1.. {
        let ptr = unsafe { ptr.cast::<[u8; 16]>().add(i) };
        let word: [u8; 16] = unsafe { ptr.read() };
        let word = std::simd::u8x16::from_array(word);
        let newline_pos = word
            .simd_eq(u8x16::splat(val))
            .to_bitmask()
            .trailing_zeros() as usize;
        if newline_pos < 16 {
            return i * 16 + newline_pos;
        }
    }
    unreachable!()
}
