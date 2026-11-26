use crate::{constants::*, Result};
use std::{
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
};

pub fn decode<T>(
    file: &Path,
    timeout_secs: u32,
    consume: impl FnOnce(u64, &mut dyn Read) -> Result<T>,
) -> Result<T> {
    let metadata = file.symlink_metadata()?;
    if !metadata.is_file() {
        return Err("Expected a local regular video file".into());
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
    let mut stdout = child.stdout.take().unwrap();
    let (done, signal) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let supervisor = scope.spawn(move || -> std::io::Result<bool> {
            let timeout = matches!(
                signal.recv_timeout(std::time::Duration::from_secs(u64::from(timeout_secs))),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            );
            let _ = child.kill();
            child.wait()?;
            Ok(timeout)
        });
        let result = (|| {
            let mut frames = Frames::new(&mut stdout);
            let mut header = [0; 12];
            frames.read_exact(&mut header)?;
            let size = u64::from_le_bytes(header[..8].try_into()?);
            if size < 8 || size.checked_add(12).is_none() {
                return Err("Invalid archive length".into());
            }
            let mut payload = frames.take(size);
            let archive = consume(size, &mut payload)?;
            if payload.limit() != 0 {
                return Err("Truncated video payload".into());
            }
            Ok(archive)
        })();
        drop(done);
        if supervisor
            .join()
            .map_err(|_| "Decoder supervisor failed")??
        {
            return Err(format!("Decoding exceeded {timeout_secs} seconds").into());
        }
        result
    })
}

struct Frames<R> {
    input: R,
    pixels: Vec<u8>,
    bytes: [u8; FRAME_BYTES],
    offset: usize,
}

impl<R: Read> Frames<R> {
    fn new(input: R) -> Self {
        Self {
            input,
            pixels: vec![0; WIDTH * HEIGHT],
            bytes: [0; FRAME_BYTES],
            offset: FRAME_BYTES,
        }
    }
}

impl<R: Read> Read for Frames<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.offset == FRAME_BYTES {
            self.input.read_exact(&mut self.pixels)?;
            for (index, byte) in self.bytes.iter_mut().enumerate() {
                let y = index / (WIDTH / (BLOCK * 8)) * BLOCK;
                let x = index % (WIDTH / (BLOCK * 8)) * BLOCK * 8;
                *byte = 0;
                for bit in 0..8 {
                    let i = y * WIDTH + x + bit * BLOCK;
                    let samples = [
                        i + 2 * WIDTH + 2,
                        i + WIDTH + 1,
                        i + WIDTH + 2,
                        i + 2 * WIDTH + 1,
                        i + 2 * WIDTH + 2,
                    ];
                    if samples
                        .iter()
                        .map(|&k| u32::from(self.pixels[k]))
                        .sum::<u32>()
                        > 128 * 5
                    {
                        *byte |= 1 << bit;
                    }
                }
            }
            self.offset = 0;
        }
        let count = output.len().min(FRAME_BYTES - self.offset);
        output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
