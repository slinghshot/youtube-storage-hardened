use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use walkdir::WalkDir;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Creates a zip archive from a file or directory
/// Returns the compressed data as bytes
pub fn create_zip(input_path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let path = Path::new(input_path);
    if !path.exists() {
        return Err(format!("Path not found: {}", input_path).into());
    }

    let buffer = Vec::new();
    let mut zip = ZipWriter::new(std::io::Cursor::new(buffer));

    let options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    if path.is_file() {
        // Single file - add it to the zip
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        zip.start_file(file_name.to_string(), options)?;

        let mut file = File::open(path)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        zip.write_all(&contents)?;

        println!("Added file to archive: {}", file_name);
    } else if path.is_dir() {
        // Directory - recursively add all files
        let base_path = path.parent().unwrap_or(path);

        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            let relative_path = entry_path
                .strip_prefix(base_path)
                .unwrap_or(entry_path)
                .to_string_lossy()
                .to_string();

            if entry_path.is_file() {
                zip.start_file(&relative_path, options)?;

                let mut file = File::open(entry_path)?;
                let mut contents = Vec::new();
                file.read_to_end(&mut contents)?;
                zip.write_all(&contents)?;

                println!("Added to archive: {}", relative_path);
            } else if entry_path.is_dir() && entry_path != path {
                // Add directory entry
                let dir_name = if relative_path.ends_with('/') {
                    relative_path.clone()
                } else {
                    format!("{}/", relative_path)
                };
                zip.add_directory(&dir_name, options)?;
            }
        }
    } else {
        return Err(format!("Unsupported path type: {}", input_path).into());
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

/// Extracts a zip archive to the specified output directory
pub fn extract_zip(zip_data: &[u8], output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output_path = Path::new(output_dir);

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_path)?;

    let cursor = std::io::Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)?;

    println!("Extracting {} files from archive...", archive.len());

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = output_path.join(file.mangled_name());

        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }

            let mut outfile = File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;

            // Set permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
                }
            }

            println!("Extracted: {}", outpath.display());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_zip_file() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "Hello, World!").unwrap();

        let zip_data = create_zip(test_file.to_str().unwrap()).unwrap();
        assert!(!zip_data.is_empty());

        let extract_dir = temp_dir.path().join("extracted");
        extract_zip(&zip_data, extract_dir.to_str().unwrap()).unwrap();

        let extracted_file = extract_dir.join("test.txt");
        assert!(extracted_file.exists());
        let contents = fs::read_to_string(&extracted_file).unwrap();
        assert_eq!(contents, "Hello, World!");
    }

    #[test]
    fn test_zip_directory() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("mydir");
        fs::create_dir(&test_dir).unwrap();
        fs::write(test_dir.join("file1.txt"), "File 1").unwrap();
        fs::write(test_dir.join("file2.txt"), "File 2").unwrap();

        let zip_data = create_zip(test_dir.to_str().unwrap()).unwrap();
        assert!(!zip_data.is_empty());

        let extract_dir = temp_dir.path().join("extracted");
        extract_zip(&zip_data, extract_dir.to_str().unwrap()).unwrap();

        assert!(extract_dir.join("mydir").join("file1.txt").exists());
        assert!(extract_dir.join("mydir").join("file2.txt").exists());
    }
}
