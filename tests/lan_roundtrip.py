"""Exercise a disposable running Compose deployment with a trusted test CA."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import zipfile

origin = os.environ["PUBLIC_ORIGIN"]
ca = Path(os.environ["CA_FILE"]).resolve()
login_file = Path(os.environ["LOGIN_FILE"]).resolve()
password = secrets.token_hex(24)
compose_dir = Path.cwd()


def compose(*args):
    subprocess.run(["docker", "compose", *args], cwd=compose_dir, check=True)


def request(route, *args, expected=200, target=None, request_origin=None):
    cmd = ["curl", "--silent", "--show-error", "--max-time", "1200", "--cacert", str(ca),
           "-b", "cookies", "-c", "cookies", "-o", target or "response", "-w", "%{http_code}",
           "-H", "Origin: " + (request_origin or origin), *args, origin + route]
    status = subprocess.check_output(cmd).decode()
    assert status == str(expected), (route, status, expected)
    return Path("response").read_bytes() if target is None else b""


def sign_in():
    request("/login", "--data-urlencode", "password=" + login_file.read_text().strip())


def upload(mode, files, secret=password):
    parts = ["--form-string", "password=" + secret, "--form-string", "mode=" + mode]
    for name, path in files:
        parts += ["-F", f"{name}=@{path};filename=upload"]
    return request("/jobs", *parts, expected=202).decode()


def wait(job, state="Ready"):
    for _ in range(1200):
        rows = json.loads(request("/jobs"))
        row = next((r for r in rows if r["id"] == job), None)
        if row and row["status"] == state:
            return
        assert not row or row["status"] != "Failed", row
        time.sleep(1)
    raise TimeoutError(job)


def download(job, target):
    request(f"/jobs/{job}/download", "--data-urlencode", "password=" + password, target=target)


def digest(name):
    with open(name, "rb") as data:
        return hashlib.file_digest(data, "sha256").digest()


with tempfile.TemporaryDirectory() as tmp:
    os.chdir(tmp)
    request("/jobs", expected=401)
    request("/login", "-d", "password=x", expected=403, request_origin="https://wrong.invalid")
    sign_in()
    Path("notes").write_bytes(secrets.token_bytes(24000))
    first = upload("create", [("file:notes", "notes")])
    wait(first)
    # The service UID itself cannot open these names after eviction.
    compose("exec", "-T", "app", "sh", "-ec",
            'test ! -e "$1/result" && test ! -e "$1/name" && test -n "$(ls -A "$1")"',
            "check-locked", "/jobs/" + first + "/vault")
    compose("restart", "app")
    for _ in range(60):
        try:
            request("/jobs", expected=401)
            break
        except (AssertionError, subprocess.CalledProcessError):
            time.sleep(1)
    sign_in()
    request(f"/jobs/{first}/download", "-d", "password=wrong", expected=403)
    download(first, "backup.mp4")
    cli = str(Path.cwd() / "youtube-storage")
    compose("cp", "app:/usr/local/bin/youtube-storage", cli)
    os.chmod(cli, 0o700)
    subprocess.run([cli, "decompress", "--file", "backup.mp4", "--output", "cli-restored",
                    "--passphrase-stdin"], input=password.encode(), check=True)
    assert digest("notes") == digest("cli-restored/notes")
    subprocess.run([cli, "compress", "--file", "notes", "--output", "cli.mp4", "--passphrase-stdin"],
                   input=password.encode(), check=True)
    cli_job = upload("restore", [("file:cli.mp4", "cli.mp4")])
    wait(cli_job)
    download(cli_job, "cli-via-web")
    assert digest("notes") == digest("cli-via-web")
    wrong = upload("restore", [("file:backup.mp4", "backup.mp4")], secret="wrong")
    wait(wrong, "Failed")
    request(f"/jobs/{wrong}/download", "-d", "password=wrong", expected=404)
    second = upload("restore", [("file:backup.mp4", "backup.mp4")])
    wait(second)
    download(second, "recovered")
    assert digest("notes") == digest("recovered")
    request(f"/jobs/{first}/delete", "-X", "POST", expected=204)
    request(f"/jobs/{first}/download", "-d", "password=x", expected=404)

    # Use an actual soundtrack and a folder containing multiple files.
    subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=duration=0.1", "tone.wav"], check=True)
    folder = upload("create", [("file:folder/a", "notes"), ("file:folder/sub/b", "notes"), ("audio", "tone.wav")])
    wait(folder)
    download(folder, "folder.mp4")
    restored = upload("restore", [("file:folder.mp4", "folder.mp4")])
    wait(restored)
    download(restored, "folder.zip")
    with zipfile.ZipFile("folder.zip") as archive:
        assert archive.read("folder/sub/b") == archive.read("folder/a") == Path("notes").read_bytes()
    # Force a test job past its deadline and restart with an incomplete job present.
    # These operations are restricted to the dedicated CI environment below.
    if os.environ.get("LAN_DISPOSABLE_TEST") == "1":
        partial = secrets.token_hex(16)
        compose("exec", "-T", "app", "sh", "-ec",
                'echo 1 > "$1/complete"; mkdir "$2"', "expire",
                "/jobs/" + restored, "/jobs/" + partial)
        compose("restart", "app")
        for _ in range(60):
            try:
                sign_in()
                break
            except (AssertionError, subprocess.CalledProcessError):
                time.sleep(1)
        request(f"/jobs/{restored}/download", "-d", "password=x", expected=404)
        compose("exec", "-T", "app", "sh", "-ec", 'test ! -e "$1" && test ! -e "$2"',
                "cleaned", "/jobs/" + restored, "/jobs/" + partial)
    print("HTTPS upload, restart, unlock, wrong passphrase, folder, soundtrack, download and deletion passed.")

    if os.getenv("RUN_ZIP64") == "1":
        # Sparse source tests ZIP64 and >4 GiB HTTP streaming with a short video.
        with open("large", "wb") as source:
            source.seek(1 << 32)
            source.write(b"ZIP64")
        large = upload("create", [("file:large", "large")])
        wait(large)
        download(large, "large.mp4")
        restored = upload("restore", [("file:large.mp4", "large.mp4")])
        wait(restored)
        download(restored, "large-restored")
        assert digest("large") == digest("large-restored")
        print("4 GiB + 5 bytes survived HTTPS upload, conversion, restore, and download.")
