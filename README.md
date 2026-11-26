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

Compression prompts for a passphrase twice on the terminal. It encrypts the complete ZIP, including entry names, using Argon2id and AES-256-GCM with a fresh random salt and nonce. Keep the passphrase in a password manager. Restore verifies authentication before creating output files. The MP4 basename, video length, and approximate backup size remain visible. Compression refuses to overwrite an existing video.

Choose a trusted output parent directory and a new destination name. On Unix, the restore root is owner-only and files are created with mode 0600. Windows permissions inherit from the parent directory. Backups support regular files and directories with portable UTF-8 names. Links and special files are unsupported. Avoid modifying the source tree during compression.

The CLI limits backups to 4,096 entries, 256 MiB of uncompressed file data and 64 MiB of encrypted payload. Decode accepts local MP4/MOV and Matroska/WebM videos up to 1 GiB and frames up to 8,294,400 pixels, with a 30-minute decode timeout. It streams fixed-size frames through FFmpeg and restricts input protocols to local files. Use a separate backup method for larger archives.

## Recover a backup made by version 0.2

```sh
youtube-storage decompress --legacy --file old.mp4 --output ./restored
youtube-storage decompress --legacy --password --file old-encrypted.mp4 --output ./restored-private
```

Use the second command for legacy ZipCrypto and enter the old password at the prompt. Legacy data lacks authenticated encryption. Recover trusted old backups into a new directory and re-encode them to migrate. The same resource limits and filename rules apply.
