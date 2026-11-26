# YouTube Storage

This Rust CLI encodes a file or folder into high-contrast video frames that can survive video recompression. Upload and download are manual. Verify that a downloaded video restores successfully before relying on it as a backup.

## Build

Install Rust 1.95 and FFmpeg with the libx264 encoder. Use a current FFmpeg installation from a trusted source and ensure its executable is first on your PATH.

```sh
cargo build --release --locked
```

## Create and restore a backup

```sh
youtube-storage compress --file ./documents
youtube-storage decompress --file ./documents.mp4 --output ./restored
```

Compression prompts for a passphrase twice on the terminal. It encrypts the complete ZIP, including entry names, using Argon2id with a fresh random salt and chunked AES-256-GCM. The authentication tags bind each chunk to its position and the complete payload length. Keep the passphrase in a password manager. Restore verifies authentication before creating output files. The MP4 basename, video length, and approximate backup size remain visible. Compression refuses to overwrite an existing video.

Choose a trusted output parent directory and a new destination name. On Unix, the restore root is owner-only and files are created with mode 0600. Windows permissions inherit from the parent directory. Backups support regular files and directories with portable UTF-8 names. Links and special files are unsupported. Avoid modifying the source tree during compression.

New backups use ZIP64 and have no fixed archive, expanded-data, or video file size cap. They support up to 4,096 entries. Archive processing uses fixed-size buffers and private temporary files in the system temporary directory. Compression needs space for the temporary ZIP and output MP4. Encrypted chunks go directly to the video encoder. Restore needs space for the temporary ZIP and extracted files. The temporary ZIP contains plaintext. Use a temporary directory on an encrypted volume when the files require encryption at rest.

Decode accepts local MP4/MOV and Matroska/WebM, with a frame limit of 8,294,400 pixels. Set `--decode-timeout-secs` to raise the default 1,800-second timeout for long jobs. Use `--max-restore-bytes` to enforce an upper bound on extracted bytes. At 30 fps each hour carries about 778 MB of payload before encryption overhead. The video platform's upload and duration limits still apply.

Existing `YTSTORE1` encrypted videos remain readable and retain their original 64 MiB encrypted payload limit.

## Recover a backup made by version 0.2

```sh
youtube-storage decompress --legacy --file old.mp4 --output ./restored
youtube-storage decompress --legacy --password --file old-encrypted.mp4 --output ./restored-private
```

Use the second command for legacy ZipCrypto and enter the old password at the prompt. Legacy data lacks authenticated encryption. Recover trusted old backups into a new directory and re-encode them to migrate. Legacy ZIPs default to 64 MiB compressed and 256 MiB extracted. An explicit `--max-restore-bytes` sets both budgets for legacy recovery.
