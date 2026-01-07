# YouTube Storage

A Rust CLI tool that encodes any file into a video that can survive YouTube's compression, allowing you to use YouTube as free cloud storage.

## How It Works

YouTube re-encodes all uploaded videos with lossy compression, which normally corrupts binary data. This tool uses a **YouTube-resilient encoding scheme**:

- **Black & White pixels only** — Maximum contrast survives compression
- **4×4 pixel blocks per bit** — Redundancy protects against artifacts  
- **Brightness threshold decoding** — Simple >128 = 1, else = 0

Each frame stores **7,200 bytes** of data at 1280×720 resolution.

## Installation

### Prerequisites
- [Rust](https://rustup.rs/) (1.70+)
- [FFmpeg](https://ffmpeg.org/) installed and in PATH

### Build

```bash
cargo build --release
```

The binary will be at `./target/release/youtube-storage`

## Usage

### Compress (Encode file → Video)

```bash
youtube-storage compress --file mydata.zip
```

Creates `mydata.mp4` ready for YouTube upload.

### Decompress (Video → Original file)

```bash
youtube-storage decompress --file downloaded.mp4 --output recovered.zip
```

Works with videos downloaded from YouTube after processing.

## Example Workflow

1. **Encode your file:**
   ```bash
   ./target/release/youtube-storage compress --file backup.zip
   ```

2. **Upload `backup.mp4` to YouTube** (unlisted recommended)

3. **When needed, download the video from YouTube**

4. **Decode back to original:**
   ```bash
   ./target/release/youtube-storage decompress --file backup.mp4 --output backup.zip
   ```

5. **Verify integrity:**
   ```bash
   md5 original.zip recovered.zip  # Should match!
   ```

## Technical Specifications

| Parameter | Value |
|-----------|-------|
| Resolution | 1280×720 (720p) |
| Frame Rate | 15 fps |
| Block Size | 4×4 pixels per bit |
| Bits per Frame | 57,600 |
| Bytes per Frame | 7,200 |
| Encoding | Binary (black/white) |
| Output Format | MP4 (H.264) |

### Storage Capacity

| File Size | Video Duration | Frames |
|-----------|----------------|--------|
| 1 MB | ~9 seconds | 139 |
| 10 MB | ~1.5 minutes | 1,389 |
| 100 MB | ~15 minutes | 13,889 |
| 1 GB | ~2.5 hours | 138,889 |

### Why It Survives YouTube

1. **Binary encoding** — Only two states (black/white), not 16 million colors
2. **Large pixel blocks** — 4×4 = 16 pixels per bit provides redundancy
3. **High contrast** — RGB(0,0,0) vs RGB(255,255,255) maximally separated
4. **Multi-point sampling** — Decoder samples 5 points per block and averages
5. **Brightness threshold** — Even if white becomes gray (200,200,200), it's still >128

## Limitations

- **Video length** — Large files create long videos (224 MB ≈ 36 minutes)
- **Processing time** — Encoding/decoding takes time for large files
- **YouTube limits** — Videos over 15 minutes require account verification
- **Not encrypted** — Data is visible as patterns; encrypt sensitive files first

## Tips

- **Encrypt first** — Use `gpg` or `zip -e` for sensitive data
- **Use unlisted** — Keep videos unlisted to avoid attention
- **Download highest quality** — Always download the highest resolution available
- **Verify checksums** — Always verify MD5/SHA256 after recovery

## License

MIT

## Disclaimer

This tool is for educational purposes. Review YouTube's Terms of Service before using for storage. The authors are not responsible for any account actions or data loss.
