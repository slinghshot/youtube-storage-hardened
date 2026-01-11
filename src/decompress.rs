use rayon::prelude::*;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

use crate::constants::{BLOCKS_X, BLOCKS_Y, BLOCK_SIZE, BYTES_PER_FRAME, HEIGHT, WIDTH};
use crate::zip_utils;

type FrameResult = Result<(usize, Vec<u8>), Box<dyn std::error::Error + Send + Sync>>;

pub fn decompress_file(input_path: &str, output_path: &str, password: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(input_path);
    if !path.exists() {
        return Err(format!("Video file not found: {}", input_path).into());
    }

    println!("Extracting frames from {}...", input_path);

    // Create temporary directory for extracted frames
    let temp_dir = TempDir::new()?;
    let frames_dir = temp_dir.path();

    // Use ffmpeg to extract frames as PNG
    // Scale to exact resolution in case YouTube changed it
    let ffmpeg_status = Command::new("ffmpeg")
        .args([
            "-i",
            input_path,
            "-vf",
            &format!("scale={}:{}", WIDTH, HEIGHT),
            "-vsync",
            "0",
            frames_dir.join("frame_%06d.png").to_str().unwrap(),
        ])
        .status()?;

    if !ffmpeg_status.success() {
        return Err("ffmpeg failed to extract frames".into());
    }

    // Count extracted frames
    let mut frame_files: Vec<_> = std::fs::read_dir(frames_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "png"))
        .collect();
    frame_files.sort_by_key(|e| e.path());

    let num_frames = frame_files.len();
    println!("Extracted {} frames", num_frames);

    // Read all frames and extract bytes in parallel using rayon
    let frames_processed = AtomicUsize::new(0);
    let frame_results: Vec<FrameResult> =
        frame_files
            .par_iter()
            .enumerate()
            .map(|(idx, entry)| {
                let frame_path = entry.path();
                let bytes = extract_bytes_from_frame(&frame_path)?;

                let count = frames_processed.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 100 == 0 || count == num_frames {
                    println!("Processed frame {}/{}", count, num_frames);
                }

                Ok((idx, bytes))
            })
            .collect();

    // Sort results by frame index and combine bytes
    let mut indexed_results: Vec<(usize, Vec<u8>)> = Vec::with_capacity(num_frames);
    for result in frame_results {
        match result {
            Ok(r) => indexed_results.push(r),
            Err(e) => return Err(e.to_string().into()),
        }
    }
    indexed_results.sort_by_key(|(idx, _)| *idx);

    let mut all_bytes: Vec<u8> = Vec::with_capacity(num_frames * BYTES_PER_FRAME);
    for (_, bytes) in indexed_results {
        all_bytes.extend_from_slice(&bytes);
    }

    // Extract the header: file size (8 bytes), fps (4 bytes)
    if all_bytes.len() < 12 {
        return Err("Invalid data: not enough bytes for header".into());
    }

    let file_size_bytes: [u8; 8] = all_bytes[0..8].try_into()?;
    let original_file_size = u64::from_le_bytes(file_size_bytes) as usize;

    let fps_bytes: [u8; 4] = all_bytes[8..12].try_into()?;
    let fps = u32::from_le_bytes(fps_bytes);

    println!("Original data size: {} bytes", original_file_size);
    println!("Original FPS: {}", fps);
    println!(
        "Video duration: {:.2} seconds ({:.2} minutes)",
        num_frames as f64 / fps as f64,
        num_frames as f64 / fps as f64 / 60.0
    );

    // Extract the actual data (skip 12 bytes for header)
    if all_bytes.len() < 12 + original_file_size {
        return Err(format!(
            "Invalid data: expected {} bytes but only have {}",
            original_file_size,
            all_bytes.len() - 12
        )
        .into());
    }

    let zip_data = &all_bytes[12..12 + original_file_size];

    // Extract the zip archive (with optional decryption)
    println!("Extracting archive to: {}", output_path);
    zip_utils::extract_zip(zip_data, output_path, password)?;

    println!("Successfully decompressed to: {}", output_path);

    Ok(())
}

fn extract_bytes_from_frame(frame_path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let img = image::open(frame_path)?.to_rgb8();

    let mut bytes = Vec::with_capacity(BYTES_PER_FRAME);
    let mut current_byte: u8 = 0;
    let mut bit_count = 0;

    for block_y in 0..BLOCKS_Y {
        for block_x in 0..BLOCKS_X {
            // Sample the center of each block for robustness
            let center_x = block_x * BLOCK_SIZE + BLOCK_SIZE / 2;
            let center_y = block_y * BLOCK_SIZE + BLOCK_SIZE / 2;

            // Also sample corners and average for even more robustness
            let mut total_brightness: u32 = 0;
            let samples = [
                (center_x, center_y),
                (block_x * BLOCK_SIZE + 1, block_y * BLOCK_SIZE + 1),
                (
                    block_x * BLOCK_SIZE + BLOCK_SIZE - 2,
                    block_y * BLOCK_SIZE + 1,
                ),
                (
                    block_x * BLOCK_SIZE + 1,
                    block_y * BLOCK_SIZE + BLOCK_SIZE - 2,
                ),
                (
                    block_x * BLOCK_SIZE + BLOCK_SIZE - 2,
                    block_y * BLOCK_SIZE + BLOCK_SIZE - 2,
                ),
            ];

            for (sx, sy) in samples {
                let pixel = img.get_pixel(sx.min(WIDTH - 1), sy.min(HEIGHT - 1));
                // Calculate brightness (grayscale value)
                let brightness = (pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32) / 3;
                total_brightness += brightness;
            }

            let avg_brightness = total_brightness / samples.len() as u32;

            // Threshold: > 128 means white (1), otherwise black (0)
            let bit = if avg_brightness > 128 { 1u8 } else { 0u8 };

            // Add bit to current byte
            current_byte |= bit << bit_count;
            bit_count += 1;

            if bit_count == 8 {
                bytes.push(current_byte);
                current_byte = 0;
                bit_count = 0;
            }
        }
    }

    Ok(bytes)
}
