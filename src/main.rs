mod compress;
mod constants;
mod decompress;
mod encryption;
mod zip_utils;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create an encrypted video backup
    Compress {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=120))]
        fps: u32,
    },
    /// Restore a backup into a new private directory
    Decompress {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Accept an old unauthenticated ZIP backup
        #[arg(long)]
        legacy: bool,
        /// Prompt for the old ZipCrypto password
        #[arg(long, requires = "legacy")]
        password: bool,
    },
}

fn password(prompt: &str) -> Result<Zeroizing<String>> {
    let value = Zeroizing::new(rpassword::prompt_password(prompt)?);
    if value.is_empty() {
        return Err("Empty password".into());
    }
    Ok(value)
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Compress { file, fps } => {
            let secret = password("New backup passphrase: ")?;
            if secret != password("Confirm passphrase: ")? {
                return Err("Passphrases differ".into());
            }
            let zip = zip_utils::create_zip(&file)?;
            let encrypted = encryption::encrypt(&zip, &secret)?;
            compress::encode(&file, fps, &encrypted)
        }
        Commands::Decompress {
            file,
            output,
            legacy,
            password: old_password,
        } => {
            let data = decompress::decode(&file)?;
            if data.starts_with(encryption::MAGIC) {
                let secret = password("Backup passphrase: ")?;
                zip_utils::restore(&encryption::decrypt(&data, &secret)?, &output, None)
            } else if legacy {
                let secret = old_password
                    .then(|| password("Legacy ZIP password: "))
                    .transpose()?;
                zip_utils::restore(&data, &output, secret.as_ref().map(|p| p.as_str()))
            } else {
                Err("Unknown or unauthenticated backup. Old backups require --legacy".into())
            }
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
