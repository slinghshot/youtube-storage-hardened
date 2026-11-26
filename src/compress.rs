use crate::{constants::*, Result};
use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

pub fn encode(
    file: &Path,
    fps: u32,
    size: u64,
    write_payload: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    size.checked_add(12).ok_or("Video payload too large")?;
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
    let mut frames = FrameWriter {
        output: child.stdin.take().unwrap(),
        pixels: vec![0; WIDTH * HEIGHT],
        bytes: [0; FRAME_BYTES],
        used: 0,
    };
    let result = (|| -> Result<()> {
        frames.write_all(&size.to_le_bytes())?;
        frames.write_all(&fps.to_le_bytes())?;
        write_payload(&mut frames)?;
        if frames.used != 0 {
            frames.bytes[frames.used..].fill(0);
            frames.write_frame()?;
        }
        Ok(())
    })();
    drop(frames);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    result?;
    if !status.success() {
        return Err("FFmpeg encoding failed".into());
    }
    temp.persist_noclobber(&output)?;
    println!("Created {}", output.display());
    Ok(())
}

struct FrameWriter<W> {
    output: W,
    pixels: Vec<u8>,
    bytes: [u8; FRAME_BYTES],
    used: usize,
}

impl<W: Write> FrameWriter<W> {
    fn write_frame(&mut self) -> io::Result<()> {
        for (y, rows) in self.pixels.chunks_exact_mut(WIDTH * BLOCK).enumerate() {
            let (row, rest) = rows.split_at_mut(WIDTH);
            for (x, pixels) in row.chunks_exact_mut(BLOCK).enumerate() {
                let bit = y * (WIDTH / BLOCK) + x;
                let byte = self.bytes[bit / 8];
                pixels.fill(if byte & (1 << (bit % 8)) == 0 { 0 } else { 255 });
            }
            for other in rest.chunks_exact_mut(WIDTH) {
                other.copy_from_slice(row);
            }
        }
        self.output.write_all(&self.pixels)
    }
}

impl<W: Write> Write for FrameWriter<W> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let count = input.len().min(FRAME_BYTES - self.used);
        self.bytes[self.used..self.used + count].copy_from_slice(&input[..count]);
        self.used += count;
        if self.used == FRAME_BYTES {
            self.write_frame()?;
            self.used = 0;
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}
