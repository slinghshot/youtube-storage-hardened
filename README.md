# YouTube Storage

A Rust CLI tool that encodes any file into a video that can survive YouTube's compression, allowing you to use YouTube as free cloud storage.

## How It Works

YouTube re-encodes all uploaded videos with lossy compression, which normally corrupts binary data. This tool uses a **YouTube-resilient encoding scheme**:

- **Black & White pixels only** — Maximum contrast survives compression
- **4×4 pixel blocks per bit** — Redundancy protects against artifacts  
- **Brightness threshold decoding** — Simple >128 = 1, else = 0

Each frame stores **7,200 bytes** of data at 1280×720 resolution.

## Process Flow

### Compression Process

```mermaid
flowchart TD
    A[📁 Input File/Directory] --> B[Create ZIP Archive]
    B --> C{Password Provided?}
    C -->|Yes| D[🔐 Encrypt with AES-256-GCM]
    C -->|No| E[Skip Encryption]
    D --> F[Add Header Metadata]
    E --> F
    F --> G[Split Data into Frames]
    G --> H[Generate Black/White Pixel Blocks]
    H --> I[Create PNG Frame Images]
    I --> J[🎬 Encode to MP4 with FFmpeg]
    J --> K[📹 Output Video File]

    subgraph Header["Header (13 bytes)"]
        F1[File Size: 8 bytes]
        F2[FPS: 4 bytes]
        F3[Encrypted Flag: 1 byte]
    end

    subgraph Encoding["Pixel Encoding"]
        H1[1 bit = 4x4 pixel block]
        H2[Bit 0 = Black RGB 0,0,0]
        H3[Bit 1 = White RGB 255,255,255]
    end
```

### Decompression Process

```mermaid
flowchart TD
    A[📹 Input Video File] --> B[🎬 Extract Frames with FFmpeg]
    B --> C[Read Pixel Blocks from Frames]
    C --> D[Sample Brightness per Block]
    D --> E[Reconstruct Binary Data]
    E --> F[Parse Header Metadata]
    F --> G{Encrypted?}
    G -->|Yes| H{Password Provided?}
    G -->|No| K[Extract ZIP Archive]
    H -->|Yes| I[🔓 Decrypt with AES-256-GCM]
    H -->|No| J[❌ Error: Password Required]
    I --> K
    K --> L[📁 Output Files/Directory]

    subgraph Decoding["Pixel Decoding"]
        D1[Sample 5 points per block]
        D2[Average brightness]
        D3["> 128 = 1, else = 0"]
    end
```

### End-to-End Workflow

```mermaid
flowchart LR
    subgraph Local["Local Machine"]
        A[📁 Files] --> B[Compress + Encrypt]
        B --> C[📹 Video]
    end

    subgraph Cloud["YouTube Cloud"]
        D[(YouTube Storage)]
    end

    subgraph Recovery["Recovery"]
        E[📹 Downloaded Video] --> F[Decompress + Decrypt]
        F --> G[📁 Restored Files]
    end

    C -->|Upload| D
    D -->|Download| E
```

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

### Compress (Encode file/directory → Video)

**Compress a file:**
```bash
youtube-storage compress --file mydata.zip
```

**Compress a directory:**
```bash
youtube-storage compress --file ./my-folder
```

**Compress with encryption:**
```bash
youtube-storage compress --file mydata.zip --password "your-secret-password"
```

Creates `mydata.mp4` (or `my-folder.mp4`) ready for YouTube upload.

### Decompress (Video → Original file/directory)

**Decompress without encryption:**
```bash
youtube-storage decompress --file downloaded.mp4 --output ./restored
```

**Decompress with decryption:**
```bash
youtube-storage decompress --file downloaded.mp4 --output ./restored --password "your-secret-password"
```

Works with videos downloaded from YouTube after processing. Files are extracted to the specified output directory.

## Example Workflow

1. **Encode your files with encryption:**
   ```bash
   ./target/release/youtube-storage compress --file ./my-documents --password "my-secure-pass"
   ```

2. **Upload `my-documents.mp4` to YouTube** (unlisted recommended)

3. **When needed, download the video from YouTube**

4. **Decode and decrypt back to original:**
   ```bash
   ./target/release/youtube-storage decompress --file my-documents.mp4 --output ./restored --password "my-secure-pass"
   ```

5. **Verify your files are intact in `./restored/my-documents/`**

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

## Security

### Built-in Encryption

This tool includes **AES-256-GCM** authenticated encryption with **Argon2id** key derivation:

- **AES-256-GCM** — Military-grade authenticated encryption
- **Argon2id** — Memory-hard password hashing (resistant to GPU attacks)
- **Random salt & nonce** — Each encryption is unique, even with the same password

When you use the `--password` option:
1. A random 16-byte salt is generated
2. Your password is derived into a 256-bit key using Argon2id
3. Data is encrypted with AES-256-GCM (includes authentication tag)
4. The encrypted data is then encoded into the video

**⚠️ Important:** If you lose your password, the data cannot be recovered!

## Limitations

- **Video length** — Large files create long videos (224 MB ≈ 36 minutes)
- **Processing time** — Encoding/decoding takes time for large files
- **YouTube limits** — Videos over 15 minutes require account verification
- **Password recovery** — No way to recover encrypted data without the password

## Tips

- **Always use encryption** — Use `--password` for any sensitive data
- **Use unlisted** — Keep videos unlisted to avoid attention
- **Download highest quality** — Always download the highest resolution available
- **Save your password securely** — Use a password manager

## License

MIT

## Disclaimer

This tool is for educational purposes. Review YouTube's Terms of Service before using for storage. The authors are not responsible for any account actions or data loss.
