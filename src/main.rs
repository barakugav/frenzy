mod hashmap;

use core::str;
use std::path::Path;

use memmap2::Mmap;

use crate::hashmap::SimpleHashMap;

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
    let measurements_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("1brc")
        .join("measurements.txt");
    let file = std::fs::File::open(measurements_file).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };
    let mut file_bytes = mmap.as_ref();

    let mut measurements = SimpleHashMap::<&str, StationSummary>::new(1000, 128.0);
    while file_bytes.len() > 0 {
        let newline_pos = file_bytes.iter().position(|&b| b == b'\n').unwrap();
        let line = &file_bytes[..newline_pos];
        file_bytes = &file_bytes[newline_pos + 1..]; // skip newline

        // format: <string: station name>;<double: measurement>
        let semicolon_pos = line.iter().position(|&b| b == b';').unwrap();
        let station_name = unsafe { std::str::from_utf8_unchecked(&line[..semicolon_pos]) };
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
    let mut measurements_sorted = measurements.iter().collect::<Vec<_>>();
    measurements_sorted.sort_by_key(|(station_name, _)| *station_name);

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
