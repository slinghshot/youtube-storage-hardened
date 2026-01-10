mod compress;
mod constants;
mod decompress;
mod encryption;
mod zip_utils;

use clap::{Parser, Subcommand};

use compress::compress_file;
use constants::DEFAULT_FPS;
use decompress::decompress_file;

#[derive(Parser)]
#[command(name = "youtube-storage")]
#[command(about = "A CLI tool for encoding files into video and decoding them back")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress a file or directory into a video with optional encryption
    Compress {
        /// The file or directory to compress
        #[arg(long)]
        file: String,
        /// Frames per second for the output video
        #[arg(long, default_value_t = DEFAULT_FPS)]
        fps: u32,
        /// Password for encryption (if not provided, data will not be encrypted)
        #[arg(long)]
        password: Option<String>,
    },
    /// Decompress a video back to the original file or directory
    Decompress {
        /// The input video file (supports .mkv, .mp4, .webm, etc.)
        #[arg(long)]
        file: String,
        /// The output directory path
        #[arg(long)]
        output: String,
        /// Password for decryption (required if the file was encrypted)
        #[arg(long)]
        password: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress { file, fps, password } => {
            if let Err(e) = compress_file(&file, fps, password.as_deref()) {
                eprintln!("Error compressing file: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Decompress { file, output, password } => {
            if let Err(e) = decompress_file(&file, &output, password.as_deref()) {
                eprintln!("Error decompressing file: {}", e);
                std::process::exit(1);
            }
        }
    }
}
