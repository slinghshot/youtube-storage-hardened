mod compress;
mod constants;
mod decompress;
mod encryption;
mod zip_utils;

use clap::{Parser, Subcommand};
use std::{
    io::{self, Seek, SeekFrom, Write},
    path::PathBuf,
};
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
        /// Maximum bytes to extract (legacy default: 256 MiB)
        #[arg(long)]
        max_restore_bytes: Option<u64>,
        /// Maximum seconds FFmpeg may run
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u32).range(1..))]
        decode_timeout_secs: u32,
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
            let size = zip.metadata()?.len();
            compress::encode(&file, fps, encryption::encrypted_size(size)?, |output| {
                encryption::encrypt(zip, size, &secret, output)
            })
        }
        Commands::Decompress {
            file,
            output,
            legacy,
            password: old_password,
            max_restore_bytes,
            decode_timeout_secs,
        } => {
            let mut zip = tempfile::tempfile()?;
            let authenticated = decompress::decode(&file, decode_timeout_secs, |size, input| {
                let mut magic = [0; 8];
                input.read_exact(&mut magic)?;
                if &magic == encryption::MAGIC {
                    let secret = password("Backup passphrase: ")?;
                    encryption::decrypt(input, size, &secret, &mut zip)?;
                    return Ok(true);
                }
                if &magic == encryption::V1_MAGIC {
                    if size > constants::LEGACY_MAX_PAYLOAD as u64 {
                        return Err("Version 1 encrypted payload exceeds 64 MiB".into());
                    }
                    let secret = password("Backup passphrase: ")?;
                    let mut data = Vec::new();
                    input.read_to_end(&mut data)?;
                    zip.write_all(&encryption::decrypt_v1(&data, &secret)?)?;
                    return Ok(true);
                }
                if !legacy || !magic.starts_with(b"PK") {
                    return Err(
                        "Unknown or unauthenticated backup. Old backups require --legacy".into(),
                    );
                }
                if size > max_restore_bytes.unwrap_or(constants::LEGACY_MAX_PAYLOAD as u64) {
                    return Err("Legacy ZIP exceeds the restore byte limit".into());
                }
                zip.write_all(&magic)?;
                io::copy(input, &mut zip)?;
                Ok(false)
            })?;
            zip.seek(SeekFrom::Start(0))?;
            let secret = (!authenticated && old_password)
                .then(|| password("Legacy ZIP password: "))
                .transpose()?;
            let limit = max_restore_bytes.unwrap_or(if authenticated {
                u64::MAX
            } else {
                constants::LEGACY_MAX_EXPANDED
            });
            zip_utils::restore(zip, &output, secret.as_ref().map(|p| p.as_str()), limit)
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
