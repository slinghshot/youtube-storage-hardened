use image::{ImageBuffer, Rgb};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use crate::constants::{
    BLOCKS_X, BLOCKS_Y, BLOCK_SIZE, BYTES_PER_FRAME, COLOR_BLACK, COLOR_WHITE, HEIGHT, WIDTH,
};
use crate::zip_utils;

pub fn compress_file(input_path: &str, fps: u32, password: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(input_path);
    if !path.exists() {
        return Err(format!("Path not found: {}", input_path).into());
    }

    // Step 1: Create a zip archive from the file or directory (with optional encryption)
    println!("Creating zip archive from: {}", input_path);
    let data = zip_utils::create_zip(input_path, password)?;

    let file_size = data.len() as u64;

    // Prepend the file size (8 bytes) and fps (4 bytes) to the data
    let mut encoded_data = Vec::with_capacity(12 + data.len());
    encoded_data.extend_from_slice(&file_size.to_le_bytes());
    encoded_data.extend_from_slice(&fps.to_le_bytes());
    encoded_data.extend_from_slice(&data);

    // Calculate number of frames needed
    let total_bytes = encoded_data.len();
    let num_frames = (total_bytes + BYTES_PER_FRAME - 1) / BYTES_PER_FRAME;

    println!(
        "Encoding into {} frames ({}x{} @ {}fps)",
        num_frames, WIDTH, HEIGHT, fps
    );
    println!(
        "Using black/white encoding with {}x{} pixel blocks",
        BLOCK_SIZE, BLOCK_SIZE
    );
    println!("{} bytes per frame", BYTES_PER_FRAME);

    // Create temporary directory for frames
    let temp_dir = TempDir::new()?;
    let frames_dir = temp_dir.path();

    // Generate frames
    for frame_idx in 0..num_frames {
        let start_byte = frame_idx * BYTES_PER_FRAME;
        let end_byte = std::cmp::min(start_byte + BYTES_PER_FRAME, total_bytes);
        let frame_data = &encoded_data[start_byte..end_byte];

        let img = create_frame_from_bytes(frame_data);
        let frame_path = frames_dir.join(format!("frame_{:06}.png", frame_idx));
        img.save(&frame_path)?;

        if (frame_idx + 1) % 100 == 0 || frame_idx == num_frames - 1 {
            println!("Generated frame {}/{}", frame_idx + 1, num_frames);
        }
    }

    // Generate output filename
    let input_stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_path = format!("{}.mp4", input_stem);

    // Use ffmpeg to create MP4 video (YouTube compatible)
    println!("Creating video with ffmpeg...");
    let ffmpeg_status = Command::new("ffmpeg")
        .args([
            "-y",
            "-framerate",
            &fps.to_string(),
            "-i",
            frames_dir.join("frame_%06d.png").to_str().unwrap(),
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-crf",
            "18", // High quality but not lossless (YouTube will re-encode anyway)
            "-pix_fmt",
            "yuv420p", // YouTube compatible pixel format
            "-s",
            &format!("{}x{}", WIDTH, HEIGHT),
            &output_path,
        ])
        .status()?;

    if !ffmpeg_status.success() {
        return Err("ffmpeg failed to create video".into());
    }

    println!("Successfully created: {}", output_path);
    println!(
        "Video duration: {:.2} seconds ({:.2} minutes)",
        num_frames as f64 / fps as f64,
        num_frames as f64 / fps as f64 / 60.0
    );
    if password.is_some() {
        println!("Note: The archive is encrypted. You will need the same password to decompress.");
    }

    Ok(())
}

fn create_frame_from_bytes(data: &[u8]) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(WIDTH, HEIGHT);

    // Fill with black first
    for pixel in img.pixels_mut() {
        *pixel = COLOR_BLACK;
    }

    let mut bit_idx = 0;

    for byte_idx in 0..BYTES_PER_FRAME {
        let byte = if byte_idx < data.len() { data[byte_idx] } else { 0 };

        // Process each bit in the byte
        for bit_pos in 0..8 {
            let bit = (byte >> bit_pos) & 1;
            let color = if bit == 1 { COLOR_WHITE } else { COLOR_BLACK };

            // Calculate block position
            let block_x = (bit_idx % BLOCKS_X as usize) as u32;
            let block_y = (bit_idx / BLOCKS_X as usize) as u32;

            if block_y < BLOCKS_Y {
                // Fill the entire block with the color
                let start_x = block_x * BLOCK_SIZE;
                let start_y = block_y * BLOCK_SIZE;

                for dy in 0..BLOCK_SIZE {
                    for dx in 0..BLOCK_SIZE {
                        img.put_pixel(start_x + dx, start_y + dy, color);
                    }
                }
            }

            bit_idx += 1;
        }
    }

    img
}
