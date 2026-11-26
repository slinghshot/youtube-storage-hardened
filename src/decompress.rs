use crate::{constants::*, Result};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

pub fn decode(file: &Path) -> Result<Vec<u8>> {
    let metadata = file.symlink_metadata()?;
    if !metadata.is_file() || metadata.len() > (1 << 30) {
        return Err("Expected a local video of at most 1 GiB".into());
    }
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            "mov,matroska,webm",
            "-threads",
            "1",
            "-max_pixels",
            "8294400",
            "-i",
        ])
        .arg(file.canonicalize()?)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "scale=1280:720",
            "-vsync",
            "0",
            "-pix_fmt",
            "gray",
            "-threads",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let (done, signal) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let supervisor = scope.spawn(move || -> std::io::Result<bool> {
            let timeout = matches!(
                signal.recv_timeout(std::time::Duration::from_secs(1800)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            );
            let _ = child.kill();
            child.wait()?;
            Ok(timeout)
        });
        let result = read_frames(stdout);
        drop(done);
        if supervisor
            .join()
            .map_err(|_| "Decoder supervisor failed")??
        {
            return Err("Decoding exceeded 30 minutes".into());
        }
        result
    })
}

fn read_frames(mut stream: impl Read) -> Result<Vec<u8>> {
    let mut frame = vec![0; WIDTH * HEIGHT];
    let mut bytes = Vec::new();
    let mut end = None;
    loop {
        stream.read_exact(&mut frame)?;
        for y in (0..HEIGHT).step_by(BLOCK) {
            for x in (0..WIDTH).step_by(BLOCK * 8) {
                let mut byte = 0;
                for bit in 0..8 {
                    let i = y * WIDTH + x + bit * BLOCK;
                    let samples = [
                        i + 2 * WIDTH + 2,
                        i + WIDTH + 1,
                        i + WIDTH + 2,
                        i + 2 * WIDTH + 1,
                        i + 2 * WIDTH + 2,
                    ];
                    if samples.iter().map(|&k| u32::from(frame[k])).sum::<u32>() > 128 * 5 {
                        byte |= 1 << bit;
                    }
                }
                bytes.push(byte);
            }
        }
        if end.is_none() {
            let size = u64::from_le_bytes(bytes[..8].try_into()?);
            if size == 0 || size > MAX_ARCHIVE as u64 {
                return Err("Invalid archive length".into());
            }
            end = Some(size as usize + 12);
        }
        let needed = end.unwrap();
        if bytes.len() >= needed {
            bytes.truncate(needed);
            bytes.drain(..12);
            return Ok(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_frames_and_out_of_range_lengths() {
        assert!(read_frames(&[][..]).is_err());
        assert!(read_frames(vec![0; WIDTH * HEIGHT].as_slice()).is_err());
        assert!(read_frames(vec![255; WIDTH * HEIGHT].as_slice()).is_err());
    }
}
