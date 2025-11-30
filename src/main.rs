use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use memmap2::Mmap;

struct StationSummary {
    min: f64,
    max: f64,
    sum: f64,
    count: u32,
}

fn main() {
    let measurements_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("1brc")
        .join("measurements.txt");
    let file = std::fs::File::open(measurements_file).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };
    let mut file_bytes = mmap.as_ref();

    let mut measurements: HashMap<&str, StationSummary> = HashMap::with_capacity(1000);
    while file_bytes.len() > 0 {
        let newline_pos = file_bytes.iter().position(|&b| b == b'\n').unwrap();
        let line = &file_bytes[..newline_pos];
        file_bytes = &file_bytes[newline_pos + 1..]; // skip newline

        // format: <string: station name>;<double: measurement>
        let semicolon_pos = line.iter().position(|&b| b == b';').unwrap();
        let station_name = std::str::from_utf8(&line[..semicolon_pos]).unwrap();
        let measurement_str = std::str::from_utf8(&line[semicolon_pos + 1..]).unwrap();
        let measurement: f64 = measurement_str.parse().unwrap();

        measurements
            .entry(station_name)
            .and_modify(|summary| {
                if measurement < summary.min {
                    summary.min = measurement;
                }
                if measurement > summary.max {
                    summary.max = measurement;
                }
                summary.sum += measurement;
                summary.count += 1;
            })
            .or_insert(StationSummary {
                min: measurement,
                max: measurement,
                sum: measurement,
                count: 1,
            });
    }
    // output
    // format: {Abha=-23.0/18.0/59.2, Abidjan=-16.2/26.0/67.3, Abéché=-10.0/29.4/69.0, Accra=-10.1/26.4/66.4, Addis Ababa=-23.7/16.0/67.0, Adelaide=-27.8/17.3/58.5, ...}
    let mut output = String::from("{");
    for (station_name, summary) in &BTreeMap::from_iter(measurements.iter()) {
        let avg = summary.sum / summary.count as f64;
        output.push_str(&format!(
            "{station_name}={:.1}/{:.1}/{:.1}, ",
            summary.min, summary.max, avg
        ));
    }
    output.pop();
    output.pop();
    output.push('}');
    println!("{}", output);
}
