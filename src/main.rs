mod compress;
mod constants;
mod decompress;

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
