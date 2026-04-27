use std::error::Error;
use std::fs::File;
use serde::Deserialize;
use rustc_hash::FxHashMap;
use rayon::prelude::*;

#[derive(Debug, Deserialize)]
struct EverythingRecord {
    #[serde(rename = "名称")]
    name: String,
    #[serde(rename = "路径")]
    path: String,
    #[serde(rename = "大小")]
    size: Option<u64>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let file_path = "test.csv";
    let file = File::open(file_path)?;

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(file);

    let folder_sizes = rdr
        .deserialize::<EverythingRecord>()
        .par_bridge()
        .filter_map(Result::ok)
        .fold(
            || FxHashMap::default(),
            |mut local_map, record| {
                let item_size = record.size.unwrap_or(0);
                let mut current_path = record.path.as_str();

                while !current_path.is_empty() {
                    if let Some(total) = local_map.get_mut(current_path) {
                        *total += item_size;
                    } else {
                        local_map.insert(current_path.to_string(), item_size);
                    }

                    match current_path.rfind(|c| c == '\\' || c == '/') {
                        Some(idx) => current_path = &current_path[..idx],
                        None => break,
                    }
                }

                if record.size.is_none() {
                    let mut full_path = String::with_capacity(record.path.len() + record.name.len() + 1);
                    full_path.push_str(&record.path);
                    if !full_path.ends_with('\\') && !full_path.ends_with('/') {
                        full_path.push('\\');
                    }
                    full_path.push_str(&record.name);
                    local_map.entry(full_path).or_insert(0);
                }

                local_map
            },
        )
        .reduce(
            || FxHashMap::default(),
            |mut map1, map2| {
                for (k, v) in map2 {
                    if let Some(total) = map1.get_mut(&k) {
                        *total += v;
                    } else {
                        map1.insert(k, v);
                    }
                }
                map1
            },
        );

    let mut results: Vec<(&String, &u64)> = folder_sizes.iter().collect();

    results.sort_unstable_by(|a, b| b.1.cmp(a.1));

    println!("{:<15} | {}", "大小 (MB)", "文件夹路径");
    println!("{:-<100}", "");
    for (path, size) in results.iter().take(20) {
        let size_mb = **size as f64 / 1024.0 / 1024.0;
        println!("{:<15.2} | {}", size_mb, path);
    }

    Ok(())
}
