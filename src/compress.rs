use crate::{constants::*, Result};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub fn encode(file: &Path, fps: u32, data: &[u8]) -> Result<()> {
    if data.len() > MAX_ARCHIVE {
        return Err("Encrypted archive exceeds 64 MiB".into());
    }
    let canonical = file.canonicalize()?;
    let mut stem = canonical.file_stem().ok_or("Missing filename")?.to_owned();
    stem.push(".mp4");
    let output = std::path::PathBuf::from(stem);
    if output.try_exists()? {
        return Err("Output video already exists".into());
    }
    let temp = tempfile::NamedTempFile::new_in(".")?;
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-protocol_whitelist",
            "pipe",
            "-f",
            "rawvideo",
            "-pixel_format",
            "gray",
            "-video_size",
            "1280x720",
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "mp4",
        ])
        .arg(temp.path().canonicalize()?)
        .stdin(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let mut payload = Vec::with_capacity(12 + data.len());
    payload.extend_from_slice(&(data.len() as u64).to_le_bytes());
    payload.extend_from_slice(&fps.to_le_bytes());
    payload.extend_from_slice(data);
    let mut frame = vec![0; WIDTH * HEIGHT];
    let result = payload.chunks(FRAME_BYTES).try_for_each(|chunk| {
        for (y, rows) in frame.chunks_exact_mut(WIDTH * BLOCK).enumerate() {
            let (row, rest) = rows.split_at_mut(WIDTH);
            for (x, pixels) in row.chunks_exact_mut(BLOCK).enumerate() {
                let bit = y * (WIDTH / BLOCK) + x;
                let byte = chunk.get(bit / 8).copied().unwrap_or(0);
                pixels.fill(if byte & (1 << (bit % 8)) == 0 { 0 } else { 255 });
            }
            for other in rest.chunks_exact_mut(WIDTH) {
                other.copy_from_slice(row);
            }
        }
        input.write_all(&frame)
    });
    drop(input);
    let status = child.wait()?;
    result?;
    if !status.success() {
        return Err("FFmpeg encoding failed".into());
    }
    temp.persist_noclobber(&output)?;
    println!("Created {}", output.display());
    Ok(())
}
