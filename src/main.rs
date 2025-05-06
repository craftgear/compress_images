use std::path::{self, Path, PathBuf};

use clap::Parser;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use std::fs::File;
use zip::ZipWriter;
use zip::write::FileOptions;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    dirname: String,
    #[arg(short, long, default_value_t = 4)]
    num_threads: usize,
}

fn check_if_directory_exists(dir: &str) -> Result<(), String> {
    let path = path::Path::new(dir);
    if !path.exists() {
        return Err(format!("Directory '{}' does not exist", dir));
    }
    if !path.is_dir() {
        return Err(format!("'{}' is not a directory", dir));
    }
    Ok(())
}

fn process_directory_recursively<F>(
    dir: &str,
    process_leaf_entry_fn: F,
) -> Result<Vec<path::PathBuf>, std::io::Error>
where
    F: for<'a> Fn(
            &'a str,
            &'a [path::PathBuf],
            &'a [path::PathBuf],
        ) -> Result<bool, std::io::Error>
        + Send
        + Sync
        + Clone,
{
    let dirents: Vec<_> = std::fs::read_dir(dir)?.collect();
    let (files, dirs): (Vec<_>, Vec<_>) = dirents
        .into_par_iter()
        .filter_map(|entry| entry.ok())
        .partition(|entry| entry.path().is_file());

    let file_paths: Vec<_> = files.iter().map(|e| e.path()).collect();
    let dir_paths: Vec<_> = dirs.iter().map(|e| e.path()).collect();

    if dirs.is_empty() {
        match process_leaf_entry_fn(dir, &file_paths, &dir_paths) {
            Ok(_) => {
                return Ok(file_paths);
            }
            Err(e) => {
                eprintln!("Error processing directory {}: {}", dir, e);
                return Err(e);
            }
        }
    }

    let subdir_files: Vec<_> = dirs
        .into_par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let process_entry = process_leaf_entry_fn.clone();
            process_directory_recursively(path.to_str().unwrap(), process_entry).ok()
        })
        .flatten()
        .collect();

    Ok(subdir_files)
}

fn is_image_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "avif" | "heic" | "svg"
        )
    } else {
        false
    }
}

fn create_zip(output_path: &str, files: &[PathBuf]) -> Result<(), std::io::Error> {
    // Create a temporary filename by appending .tmp to the output path
    let temp_path = format!("{}.tmp", output_path);
    println!("Creating temporary zip file at: {}", temp_path);

    let file = File::create(&temp_path)?;
    let mut zip = ZipWriter::new(file);

    // Use Bzip2 compression for better ratio while maintaining reasonable speed
    let options = FileOptions::<()>::default().compression_method(zip::CompressionMethod::Deflated);

    for path in files {
        let file_name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid file name")
        })?;

        zip.start_file(file_name, options)?;
        let mut file = File::open(path)?;
        std::io::copy(&mut file, &mut zip)?;
    }

    zip.finish()?;

    // Rename the temporary file to the final output path
    println!("Renaming to final path: {}", output_path);
    std::fs::rename(temp_path, output_path)?;

    Ok(())
}

fn process_leaf_directory(
    dir: &str,
    files: &[path::PathBuf],
    dirs: &[path::PathBuf],
) -> Result<bool, std::io::Error> {
    if !dirs.is_empty() {
        return Ok(false);
    }

    println!("Processing directory: {}", dir);

    let img_files: Vec<_> = files
        .iter()
        .filter(|path| is_image_file(path))
        .cloned()
        .collect();
    let other_files: Vec<_> = files
        .iter()
        .filter(|path| !is_image_file(path))
        .cloned()
        .collect();

    if !files.is_empty() && img_files.len() > other_files.len() {
        let dir_path = path::Path::new(dir);
        let dir_name = dir_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let parent_dir = dir_path.parent().and_then(|p| p.to_str()).unwrap_or(".");
        let mut zip_path = format!("{}/{}.zip", parent_dir, dir_name);
        let mut counter = 1;
        // Find a non-conflicting path by adding (1), (2), etc. if needed
        while std::path::Path::new(&zip_path).exists() {
            zip_path = format!("{}/{}({}).zip", parent_dir, dir_name, counter);
            counter += 1;
        }

        if let Err(e) = create_zip(&zip_path, &files) {
            eprintln!("Failed to create zip file: {}", e);
            return Err(e);
        }

        // After creating the zip file, delete the original directory
        println!("Deleting directory: {}", dir);
        match std::fs::remove_dir_all(dir) {
            Ok(_) => println!("Directory deleted successfully"),
            Err(e) => {
                eprintln!("Failed to delete directory: {}", e);
                return Err(e);
            }
        }
    }

    Ok(true)
}

fn main() {
    let args = Args::parse();
    let num_threads = args.num_threads;

    ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build_global()
        .unwrap();

    if let Err(e) = check_if_directory_exists(&args.dirname) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    match process_directory_recursively(&args.dirname, process_leaf_directory) {
        Ok(files) => {
            println!("Total files processed: {}", files.len());

            println!("Found {} image files", files.len());
        }
        Err(e) => {
            eprintln!("Failed to read directory: {}", e);
            std::process::exit(1);
        }
    }
}
