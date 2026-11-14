use std::path::{Path, PathBuf};
use unityfs::export::{ExportOptions, export_bundle_file};

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for e in std::fs::read_dir(path).unwrap().flatten() {
            collect(&e.path(), out);
        }
    } else {
        out.push(path.to_path_buf());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out_dir = PathBuf::from(&args[0]);
    let mut files = Vec::new();
    for a in &args[1..] {
        collect(Path::new(a), &mut files);
    }
    let options = ExportOptions { by_file: std::env::var("FLAT").is_err(), metadata: false, objects: None };
    let t = std::time::Instant::now();
    let mut objects = 0;
    for f in &files {
        match export_bundle_file(f, &out_dir, &options) {
            Ok(n) => objects += n,
            Err(e) => eprintln!("{}", e),
        }
    }
    println!("{} files, {} objects in {} ms", files.len(), objects, t.elapsed().as_millis());
}
