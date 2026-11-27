# Run on your Linux LAN

Docker Compose runs the Rust service and FFmpeg behind Caddy. Only Caddy publishes a port. A browser can upload a file or a folder and optional soundtrack, retrieve the MP4, or submit an MP4 and download the recovered file or ZIP. Browser folder selection preserves file paths; browsers omit empty directories.

## Prepare the host

Use a Linux host with Docker Compose and a dedicated private directory on a local ext4 filesystem with the **encrypt** feature enabled (or another filesystem supporting fscrypt v2). A bind mount must expose the actual filesystem. A directory inside Docker's overlay filesystem will fail the startup probe. On ext4, inspect the mount with `findmnt -T /srv/video-backup` and the backing device with `sudo tune2fs -l DEVICE`. Provision encryption support using your distribution's fscrypt instructions before continuing. The service sets its own per-job policies on empty directories; the jobs parent must have no existing fscrypt policy.

These examples assume rootful Docker without user namespace remapping. For remapped IDs use the host UID corresponding to container UID 10001. Disable swap or use encrypted swap. Disable host crash dumps that collect service memory. Core dumps are disabled in the app container. A trusted host administrator can read data while a job or download is unlocked.

Run from the repository root after mounting the desired filesystem:

```sh
sudo install -d -m 0700 /srv/video-backup
sudo install -d -m 0700 -o 10001 -g 10001 /srv/video-backup/jobs
sudo sh -c 'umask 077; set -C; openssl rand -hex 24 > /srv/video-backup/login'
sudo chown 10001:10001 /srv/video-backup/login
cp .env.example .env
```

Set `LAN_IP` to the server's LAN IPv4 address and `LAN_HOST` to a name resolving to that address on your devices, or the IP itself. Set `JOB_DIR` and `LOGIN_FILE` to the provisioned paths. Keep the firewall limited to your LAN. The example uses TCP 8443. Start the stack with `docker compose up -d --build`. Inspect startup with `docker compose logs app https`.

Caddy issues the server certificate from a private CA stored in its persistent data volume. Export the **public root certificate** over a trusted channel:

```sh
docker compose cp https:/data/caddy/pki/authorities/local/root.crt ./video-backups-root.crt
openssl x509 -in video-backups-root.crt -noout -fingerprint -sha256
```

Verify its fingerprint and install it as a trusted CA on each client device (some browsers use a separate trust store). Open `https://LAN_HOST:8443`, substituting the configured hostname, then sign in with the contents of the login file. Enable Docker at boot. Keep the Caddy data volume private and persistent; it contains the signing key.

## Jobs and storage

Use the service password to sign in and a separate backup passphrase for each job. The server unlocks a new fscrypt directory **before writing uploaded bytes**. Uploads, media tools' work files, names, and downloaded results reside under that policy. The browser and Linux host handle plaintext in memory. A random per-job filesystem key is wrapped with a passphrase-derived key using Argon2id and authenticated encryption. The outer metadata contains the opaque job ID, wrapped key, kernel key identifier, and completion/expiry state. Filesystem encryption still exposes sizes, timestamps and entry counts. Caddy streams request and response bodies without disk spooling.

Keep the page open during upload. Once accepted, the server finishes the conversion even if the browser closes. Return to Jobs and enter the backup passphrase to download over HTTPS. A single top-level file downloads directly; a folder downloads as a ZIP. Completed downloads last 24 hours, and Delete removes them sooner. Failed jobs retain a generic failure status for a day. The service allows one upload/conversion at a time. Restarts discard interrupted jobs, revoke any leftover keys, and retain completed results until expiry. Sessions expire after 24 hours or a restart.

Defaults are 16 GiB total upload, 32 GiB extracted output, 2 GiB free-space reserve, and 86,400 seconds for a conversion stage. Set `MAX_UPLOAD_BYTES`, `MAX_RESTORE_BYTES`, `RESERVE_BYTES`, and `DECODE_TIMEOUT_SECS` in `.env` before running Compose. Uploads time out after four hours. Budget space for inputs, intermediate ZIPs, working video, and results. The worker periodically checks free space. Use a dedicated filesystem or host quota for a hard disk allocation boundary. The media format and desktop CLI have their own independent limits.

The app fails startup if it cannot install a v2 policy, write a probe, evict the key and confirm the locked path is unreadable. Check filesystem support, volume ownership, Linux key quota, and container security policy if the check fails. Use a dedicated service UID. Retain the passphrase separately; the service has no recovery key.

Caddy overwrites the client IP header on each request. The app accepts that header only from the proxy identified by `TRUSTED_PROXY_HOST` on the private backend network. Ten failed sign-ins from the same client IP throttle that address for one minute. Devices sharing an address also share this limit.

A download releases its vault key when the server finishes reading the result, when a stalled transfer stops accepting data for 90 seconds, or when the job reaches its 24-hour expiry. A stalled download can be restarted before expiry. The service allows two concurrent download passphrase checks and returns HTTP 429 when both slots are busy.
