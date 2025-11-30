use std::{collections::HashMap, io::BufRead, path::Path};

fn main() {
    // measurements.txt
    let measurements_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("1brc")
        .join("measurements.txt");
    // read line by line, without reading the whole file into memory
    let file = std::fs::File::open(measurements_file).unwrap();
    let reader = std::io::BufReader::new(file);
    struct StationSummary {
        min: f64,
        max: f64,
        sum: f64,
        count: u32,
    }
    let mut measurements: HashMap<String, StationSummary> = HashMap::new();
    for line in reader.lines() {
        let line = line.unwrap();
        // format: <string: station name>;<double: measurement>
        let parts: Vec<&str> = line.split(';').collect();
        let station_name = parts[0];
        let measurement: f64 = parts[1].parse().unwrap();
        measurements
            .entry(station_name.to_string())
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
    for (station_name, summary) in &measurements {
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
