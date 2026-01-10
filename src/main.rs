use clap::{Parser, Subcommand};
use image::{ImageBuffer, Rgb};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const DEFAULT_FPS: u32 = 30;

// Block size for each bit (larger = more resilient to compression)
const BLOCK_SIZE: u32 = 4; // 4x4 pixel block per bit

// Calculate blocks per frame
const BLOCKS_X: u32 = WIDTH / BLOCK_SIZE;   // 320 blocks horizontally
const BLOCKS_Y: u32 = HEIGHT / BLOCK_SIZE;  // 180 blocks vertically
const BITS_PER_FRAME: usize = (BLOCKS_X * BLOCKS_Y) as usize; // 57,600 bits
const BYTES_PER_FRAME: usize = BITS_PER_FRAME / 8; // 7,200 bytes

// Black and white colors
const COLOR_BLACK: Rgb<u8> = Rgb([0, 0, 0]);     // Bit 0
const COLOR_WHITE: Rgb<u8> = Rgb([255, 255, 255]); // Bit 1

#[derive(Parser)]
#[command(name = "youtube-storage")]
#[command(about = "A CLI tool for encoding files into video and decoding them back")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress a file into a video
    Compress {
        /// The file to compress
        #[arg(long)]
        file: String,
        /// Frames per second for the output video
        #[arg(long, default_value_t = DEFAULT_FPS)]
        fps: u32,
    },
    /// Decompress a video back to the original file
    Decompress {
        /// The input video file (supports .mkv, .mp4, .webm, etc.)
        #[arg(long)]
        file: String,
        /// The output file path
        #[arg(long)]
        output: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress { file, fps } => {
            if let Err(e) = compress_file(&file, fps) {
                eprintln!("Error compressing file: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Decompress { file, output } => {
            if let Err(e) = decompress_file(&file, &output) {
                eprintln!("Error decompressing file: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn compress_file(input_path: &str, fps: u32) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(input_path);
    if !path.exists() {
        return Err(format!("File not found: {}", input_path).into());
    }

    // Read the input file
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    let file_size = data.len() as u64;
    println!("Read {} bytes from {}", file_size, input_path);

    // Prepend the file size (8 bytes) to the data
    let mut encoded_data = Vec::with_capacity(8 + data.len());
    encoded_data.extend_from_slice(&file_size.to_le_bytes());
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
            "18",  // High quality but not lossless (YouTube will re-encode anyway)
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

fn decompress_file(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
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

    // Read all frames and extract bytes
    let mut all_bytes: Vec<u8> = Vec::new();

    for (idx, entry) in frame_files.iter().enumerate() {
        let frame_path = entry.path();
        let bytes = extract_bytes_from_frame(&frame_path)?;
        all_bytes.extend_from_slice(&bytes);

        if (idx + 1) % 100 == 0 || idx == num_frames - 1 {
            println!("Processed frame {}/{}", idx + 1, num_frames);
        }
    }

    // Extract the original file size from the first 8 bytes
    if all_bytes.len() < 8 {
        return Err("Invalid data: not enough bytes for file size header".into());
    }

    let file_size_bytes: [u8; 8] = all_bytes[0..8].try_into()?;
    let original_file_size = u64::from_le_bytes(file_size_bytes) as usize;

    println!("Original file size: {} bytes", original_file_size);

    // Extract the actual file data
    if all_bytes.len() < 8 + original_file_size {
        return Err(format!(
            "Invalid data: expected {} bytes but only have {}",
            original_file_size,
            all_bytes.len() - 8
        )
        .into());
    }

    let file_data = &all_bytes[8..8 + original_file_size];

    // Write to output file
    let mut output_file = File::create(output_path)?;
    output_file.write_all(file_data)?;

    println!("Successfully decompressed to: {}", output_path);
    println!("Output file size: {} bytes", original_file_size);

    Ok(())
}

fn extract_bytes_from_frame(frame_path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
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
                (block_x * BLOCK_SIZE + BLOCK_SIZE - 2, block_y * BLOCK_SIZE + 1),
                (block_x * BLOCK_SIZE + 1, block_y * BLOCK_SIZE + BLOCK_SIZE - 2),
                (block_x * BLOCK_SIZE + BLOCK_SIZE - 2, block_y * BLOCK_SIZE + BLOCK_SIZE - 2),
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
