mod vault;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as RoutePath, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use fs2::FileExt;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, File},
    io::{self, Cursor},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};
use zeroize::Zeroizing;

type Shared = Arc<Service>;
type Web<T> = Result<T, StatusCode>;
const INVALID: StatusCode = StatusCode::BAD_REQUEST;
const INTERNAL: StatusCode = StatusCode::INTERNAL_SERVER_ERROR;
const SECRET_LIMIT: usize = 4096;
const DAY: u64 = 86400;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random<const N: usize>() -> [u8; N] {
    let mut b = [0; N];
    OsRng.fill_bytes(&mut b);
    b
}
fn setting(name: &str, default: u64) -> crate::Result<u64> {
    Ok(match env::var(name) {
        Ok(s) => s.parse()?,
        Err(_) => default,
    })
}

struct Service {
    root: PathBuf,
    mount: File,
    origin: String,
    login: Zeroizing<Vec<u8>>,
    sessions: Mutex<HashMap<String, u64>>,
    attempts: Mutex<Vec<u64>>,
    busy: Mutex<HashSet<String>>,
    stages: Mutex<HashMap<String, &'static str>>,
    worker: Arc<Semaphore>,
    upload: u64,
    restore: u64,
    reserve: u64,
    timeout: u32,
    _instance: File,
}
#[derive(Serialize, Deserialize)]
struct KeyInfo {
    identifier: [u8; 16],
    wrapped: Vec<u8>,
}
fn key_info(dir: &Path) -> io::Result<KeyInfo> {
    Ok(serde_json::from_slice(&fs::read(dir.join("key"))?)?)
}
fn room(s: &Service) -> Web<()> {
    if fs2::available_space(&s.root).map_err(|_| INTERNAL)? <= s.reserve {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    Ok(())
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
struct Lease {
    s: Shared,
    id: String,
    locked_key: Option<[u8; 16]>,
    discard: bool,
}
impl Lease {
    fn take(s: Shared, id: String) -> Web<Self> {
        if !valid_id(&id) {
            return Err(INVALID);
        }
        if !s.busy.lock().unwrap().insert(id.clone()) {
            return Err(StatusCode::CONFLICT);
        }
        Ok(Self {
            s,
            id,
            locked_key: None,
            discard: false,
        })
    }
    fn dir(&self) -> PathBuf {
        self.s.root.join(&self.id)
    }
    fn data(&self) -> PathBuf {
        self.dir().join("vault")
    }
    fn unlock(&mut self, key: &[u8; 64]) -> Web<()> {
        vault::unlock(&self.s.mount, key).map_err(|_| INTERNAL)?;
        self.locked_key = Some(vault::identifier(key));
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(id) = self.locked_key {
            if vault::lock(&self.s.mount, &id).is_err() {
                eprintln!("Key eviction failed; stopping for recovery");
                std::process::abort()
            }
        }
        if self.discard {
            if let Err(error) = fs::remove_dir_all(self.dir()) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!("Job cleanup will be retried");
                }
            }
        }
        self.s.stages.lock().unwrap().remove(&self.id);
        self.s.busy.lock().unwrap().remove(&self.id);
    }
}
fn create_vault(job: &mut Lease, pass: &str) -> Web<()> {
    fs::create_dir(job.dir()).map_err(|_| INTERNAL)?;
    job.discard = true;
    fs::create_dir(job.data()).map_err(|_| INTERNAL)?;
    let key = Zeroizing::new(random::<64>());
    let mut wrapped = Vec::new();
    crate::encryption::encrypt(&key[..], 64, pass, &mut wrapped).map_err(|_| INTERNAL)?;
    let info = KeyInfo {
        identifier: vault::identifier(&key),
        wrapped,
    };
    let mut meta = File::create_new(job.dir().join("key.new")).map_err(|_| INTERNAL)?;
    serde_json::to_writer(&mut meta, &info).map_err(|_| INTERNAL)?;
    meta.sync_all().map_err(|_| INTERNAL)?;
    fs::rename(job.dir().join("key.new"), job.dir().join("key")).map_err(|_| INTERNAL)?;
    File::open(job.dir())
        .and_then(|d| d.sync_all())
        .map_err(|_| INTERNAL)?;
    job.unlock(&key)?;
    vault::apply(&job.data(), &info.identifier).map_err(|_| INTERNAL)
}

fn completed(dir: &Path) -> Option<u64> {
    fs::read_to_string(dir.join("complete")).ok()?.parse().ok()
}
fn failed(dir: &Path) -> Option<u64> {
    fs::read_to_string(dir.join("failure")).ok()?.parse().ok()
}
fn cleanup(s: &Shared, startup: bool) -> crate::Result<()> {
    for item in fs::read_dir(&s.root)? {
        let id = item?.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) {
            continue;
        }
        let Ok(mut job) = Lease::take(s.clone(), id) else {
            continue;
        };
        if startup && job.dir().join("key").exists() {
            vault::lock(&s.mount, &key_info(&job.dir())?.identifier)?;
        }
        job.discard = completed(&job.dir())
            .or_else(|| failed(&job.dir()))
            .is_none_or(|expires| expires <= now());
    }
    Ok(())
}

pub fn serve() -> crate::Result<()> {
    let origin = env::var("PUBLIC_ORIGIN")?;
    let parsed = url::Url::parse(&origin)?;
    if parsed.scheme() != "https"
        || parsed.host().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("PUBLIC_ORIGIN must be an HTTPS origin".into());
    }
    let origin = parsed.origin().ascii_serialization();
    let root = PathBuf::from(env::var("JOB_ROOT")?).canonicalize()?;
    let instance = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".service-lock"))?;
    instance.try_lock_exclusive()?;
    let mut login = Zeroizing::new(fs::read(env::var("LOGIN_SECRET_FILE")?)?);
    while login.last().is_some_and(|b| matches!(b, b'\r' | b'\n')) {
        login.pop();
    }
    if login.len() < 16 || login.len() > SECRET_LIMIT {
        return Err("Service password must be 16–4096 bytes".into());
    }
    let s = Arc::new(Service {
        mount: File::open(&root)?,
        root,
        origin,
        login,
        sessions: Default::default(),
        attempts: Default::default(),
        busy: Default::default(),
        stages: Default::default(),
        worker: Arc::new(Semaphore::new(1)),
        upload: setting("MAX_UPLOAD_BYTES", 16 << 30)?,
        restore: setting("MAX_RESTORE_BYTES", 32 << 30)?,
        reserve: setting("RESERVE_BYTES", 2 << 30)?,
        timeout: setting("DECODE_TIMEOUT_SECS", 86400)?.try_into()?,
        _instance: instance,
    });
    cleanup(&s, true)?;
    {
        let mut probe = Lease::take(s.clone(), hex::encode(random::<16>())).unwrap();
        create_vault(&mut probe, &hex::encode(random::<32>()))
            .map_err(|_| "fscrypt v2 setup failed; refusing to accept uploads")?;
        fs::write(probe.data().join("probe"), b"probe")?;
        let id = probe.locked_key.unwrap();
        vault::lock(&s.mount, &id)?;
        probe.locked_key = None;
        if fs::read(probe.data().join("probe")).is_ok() {
            return Err("Locked probe was readable".into());
        }
    }
    tokio::runtime::Runtime::new()?.block_on(async move {
        let gc = s.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if cleanup(&gc, false).is_err() {
                    eprintln!("Retention cleanup failed");
                }
            }
        });
        let app = Router::new()
            .route("/", get(|| async { Html(include_str!("index.html")) }))
            .route(
                "/app.js",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/javascript")],
                        include_str!("app.js"),
                    )
                }),
            )
            .route("/login", post(sign_in).layer(DefaultBodyLimit::max(16384)))
            .route("/logout", post(logout))
            .route("/jobs", get(jobs).post(upload))
            .route(
                "/jobs/{id}/download",
                post(download).layer(DefaultBodyLimit::max(16384)),
            )
            .route("/jobs/{id}/delete", post(delete_job))
            .layer(DefaultBodyLimit::max(
                usize::try_from(s.upload.saturating_add(16 << 20)).unwrap_or(usize::MAX),
            ))
            .layer(middleware::from_fn_with_state(s.clone(), gate))
            .with_state(s);
        axum::serve(tokio::net::TcpListener::bind("0.0.0.0:8080").await?, app).await
    })?;
    Ok(())
}

fn cookie(h: &HeaderMap) -> &str {
    h.get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .map(str::trim)
        .find_map(|v| v.strip_prefix("__Host-ys="))
        .unwrap_or("")
}
async fn gate(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if req.method() != "GET"
        && req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != Some(&s.origin)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !matches!(path, "/" | "/app.js" | "/login")
        && !s
            .sessions
            .lock()
            .unwrap()
            .get(cookie(req.headers()))
            .is_some_and(|v| *v > now())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut r = next.run(req).await;
    const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("content-security-policy", CSP),
    ] {
        r.headers_mut().insert(name, value.parse().unwrap());
    }
    r
}
#[derive(Deserialize)]
struct Pass {
    password: String,
}
async fn sign_in(State(s): State<Shared>, Form(p): Form<Pass>) -> Web<Response> {
    let pass = Zeroizing::new(p.password);
    let mut attempts = s.attempts.lock().unwrap();
    attempts.retain(|t| t + 60 > now());
    if attempts.len() >= 10 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    if !bool::from(pass.as_bytes().ct_eq(&s.login)) {
        attempts.push(now());
        return Err(StatusCode::UNAUTHORIZED);
    }
    let token = hex::encode(random::<32>());
    let mut sessions = s.sessions.lock().unwrap();
    sessions.retain(|_, t| *t > now());
    if sessions.len() >= 64 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    sessions.insert(token.clone(), now() + DAY);
    Ok((
        [(
            header::SET_COOKIE,
            format!("__Host-ys={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=86400"),
        )],
        StatusCode::OK.to_string(),
    )
        .into_response())
}
async fn logout(State(s): State<Shared>, h: HeaderMap) -> impl IntoResponse {
    s.sessions.lock().unwrap().remove(cookie(&h));
    [(
        header::SET_COOKIE,
        "__Host-ys=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
    )]
}
#[derive(Serialize)]
struct Row {
    id: String,
    status: String,
    expires: Option<u64>,
}
async fn jobs(State(s): State<Shared>) -> Web<Json<Vec<Row>>> {
    let stages = s.stages.lock().unwrap();
    let mut rows = Vec::new();
    for e in fs::read_dir(&s.root).map_err(|_| INTERNAL)?.flatten() {
        let id = e.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) {
            continue;
        }
        let expires = completed(&e.path());
        if let Some(stage) = stages.get(&id) {
            rows.push(Row {
                id,
                status: stage.to_string(),
                expires: None,
            })
        } else if let Some(t) = failed(&e.path()).filter(|v| *v > now()) {
            rows.push(Row {
                id,
                status: "Failed".into(),
                expires: Some(t),
            })
        } else if expires.is_some_and(|t| t > now()) {
            rows.push(Row {
                id,
                status: "Ready".into(),
                expires,
            })
        }
    }
    rows.sort_by_key(|r| r.expires);
    Ok(Json(rows))
}
async fn delete_job(State(s): State<Shared>, RoutePath(id): RoutePath<String>) -> Web<StatusCode> {
    let job = Lease::take(s, id)?;
    if !job.dir().exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    fs::remove_dir_all(job.dir()).map_err(|_| INTERNAL)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn small(field: axum::extract::multipart::Field<'_>) -> Web<Zeroizing<String>> {
    let mut field = field;
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = field.chunk().await.map_err(|_| INVALID)? {
        if bytes.len() + chunk.len() > SECRET_LIMIT {
            return Err(INVALID);
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| INVALID)?;
    if text.is_empty() {
        return Err(INVALID);
    }
    Ok(Zeroizing::new(text.to_owned()))
}
async fn upload(State(s): State<Shared>, mut parts: Multipart) -> Web<(StatusCode, String)> {
    let permit = s
        .worker
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::CONFLICT)?;
    room(&s)?;
    let id = hex::encode(random::<16>());
    let mut job = Lease::take(s.clone(), id.clone())?;
    let (pass, mode, source, audio) = tokio::time::timeout(Duration::from_secs(4 * 3600), async {
        let field = parts
            .next_field()
            .await
            .map_err(|_| INVALID)?
            .ok_or(INVALID)?;
        if field.name() != Some("password") {
            return Err(INVALID);
        }
        let pass = small(field).await?;
        let field = parts
            .next_field()
            .await
            .map_err(|_| INVALID)?
            .ok_or(INVALID)?;
        if field.name() != Some("mode") {
            return Err(INVALID);
        }
        let mode = small(field).await?.to_string();
        if !matches!(mode.as_str(), "create" | "restore") {
            return Err(INVALID);
        }
        create_vault(&mut job, &pass)?;
        s.stages.lock().unwrap().insert(id.clone(), "Uploading");
        let data = job.data();
        let inputs = data.join("inputs");
        fs::create_dir(&inputs).map_err(|_| INTERNAL)?;
        let mut root = None;
        let mut audio = None;
        let mut files = 0;
        let mut total = 0u64;
        while let Some(mut field) = parts.next_field().await.map_err(|_| INVALID)? {
            let name = field.name().ok_or(INVALID)?;
            let path = if name == "audio" && mode == "create" && audio.is_none() {
                let path = data.join("soundtrack");
                audio = Some(path.clone());
                path
            } else {
                let relative = name.strip_prefix("file:").ok_or(INVALID)?;
                crate::zip_utils::portable_name(relative).map_err(|_| INVALID)?;
                if relative.ends_with('/')
                    || (mode == "restore" && (files > 0 || relative.contains('/')))
                {
                    return Err(INVALID);
                }
                let top = relative.split('/').next().ok_or(INVALID)?;
                if root.get_or_insert_with(|| inputs.join(top)) != &inputs.join(top) {
                    return Err(INVALID);
                }
                files += 1;
                if files > crate::constants::MAX_ENTRIES {
                    return Err(INVALID);
                }
                inputs.join(relative)
            };
            fs::create_dir_all(path.parent().ok_or(INVALID)?).map_err(|_| INVALID)?;
            let mut out = tokio::fs::File::create_new(path)
                .await
                .map_err(|_| INVALID)?;
            while let Some(chunk) = tokio::time::timeout(Duration::from_secs(90), field.chunk())
                .await
                .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
                .map_err(|_| INVALID)?
            {
                total = total.checked_add(chunk.len() as u64).ok_or(INVALID)?;
                if total > s.upload {
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                room(&s)?;
                out.write_all(&chunk)
                    .await
                    .map_err(|_| StatusCode::INSUFFICIENT_STORAGE)?;
            }
            out.flush().await.map_err(|_| INTERNAL)?;
        }
        Ok((pass, mode, root.ok_or(INVALID)?, audio))
    })
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)??;
    s.stages.lock().unwrap().insert(
        id.clone(),
        if mode == "create" {
            "Encoding"
        } else {
            "Restoring"
        },
    );
    tokio::spawn(async move {
        let _permit = permit;
        if process(&job, &pass, &mode, &source, audio.as_deref())
            .await
            .is_ok()
        {
            if fs::write(job.dir().join("pending"), (now() + DAY).to_string())
                .and_then(|_| fs::rename(job.dir().join("pending"), job.dir().join("complete")))
                .is_ok()
            {
                job.discard = false;
            }
        } else {
            // Retain only a generic status after deleting all input and output.
            if fs::remove_dir_all(job.data()).is_ok()
                && fs::write(job.dir().join("failure"), (now() + DAY).to_string()).is_ok()
            {
                job.discard = false;
            }
        }
        drop(job);
    });
    Ok((StatusCode::ACCEPTED, id))
}

async fn run_child(job: &Lease, args: &[&std::ffi::OsStr], pass: &str) -> Web<()> {
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new(env::current_exe().map_err(|_| INTERNAL)?);
    cmd.args(args)
        .current_dir(job.data())
        .env("TMPDIR", job.data())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = cmd.spawn().map_err(|_| INTERNAL)?;
    let group = nix::unistd::Pid::from_raw(child.id().ok_or(INTERNAL)? as i32);
    let mut stdin = child.stdin.take().ok_or(INTERNAL)?;
    let sent = stdin.write_all(pass.as_bytes()).await;
    drop(stdin);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(job.s.timeout) + 60);
    loop {
        if sent.is_err() || room(&job.s).is_err() || tokio::time::Instant::now() >= deadline {
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
            let _ = child.wait().await;
            return Err(INTERNAL);
        }
        tokio::select! {status=child.wait()=>return if status.map_err(|_|INTERNAL)?.success(){Ok(())}else{Err(INVALID)},_=tokio::time::sleep(Duration::from_millis(250))=>{}}
    }
}
async fn process(
    job: &Lease,
    pass: &str,
    mode: &str,
    source: &Path,
    audio: Option<&Path>,
) -> Web<()> {
    let data = job.data();
    let result = data.join("result");
    let filename = if mode == "create" {
        let mut args = vec![
            "compress".as_ref(),
            "--file".as_ref(),
            source.as_os_str(),
            "--output".as_ref(),
            result.as_os_str(),
            "--passphrase-stdin".as_ref(),
        ];
        if let Some(a) = audio {
            args.extend(["--audio".as_ref(), a.as_os_str()]);
        }
        run_child(job, &args, pass).await?;
        format!(
            "{}.mp4",
            source.file_stem().ok_or(INVALID)?.to_string_lossy()
        )
    } else {
        let dest = data.join("restored");
        run_child(
            job,
            &[
                "decompress".as_ref(),
                "--file".as_ref(),
                source.as_os_str(),
                "--output".as_ref(),
                dest.as_os_str(),
                "--max-restore-bytes".as_ref(),
                job.s.restore.to_string().as_ref(),
                "--decode-timeout-secs".as_ref(),
                job.s.timeout.to_string().as_ref(),
                "--passphrase-stdin".as_ref(),
            ],
            pass,
        )
        .await?;
        let mut entries = fs::read_dir(&dest).map_err(|_| INTERNAL)?;
        let first = entries
            .next()
            .transpose()
            .map_err(|_| INTERNAL)?
            .ok_or(INVALID)?;
        let root = if entries.next().is_none() {
            first.path()
        } else {
            dest.clone()
        };
        let name = root.file_name().ok_or(INVALID)?.to_string_lossy();
        let filename = if root.is_file() {
            fs::rename(&root, &result).map_err(|_| INTERNAL)?;
            name.into_owned()
        } else {
            run_child(
                job,
                &["lan-pack".as_ref(), root.as_os_str(), result.as_os_str()],
                "",
            )
            .await?;
            format!("{name}.zip")
        };
        fs::remove_dir_all(dest).map_err(|_| INTERNAL)?;
        filename
    };
    fs::write(data.join("name"), filename).map_err(|_| INTERNAL)?;
    fs::remove_dir_all(data.join("inputs")).map_err(|_| INTERNAL)?;
    if let Some(path) = audio {
        fs::remove_file(path).map_err(|_| INTERNAL)?;
    }
    Ok(())
}
async fn download(
    State(s): State<Shared>,
    RoutePath(id): RoutePath<String>,
    Form(p): Form<Pass>,
) -> Web<Response> {
    let pass = Zeroizing::new(p.password);
    if pass.is_empty() || pass.len() > SECRET_LIMIT {
        return Err(INVALID);
    }
    let mut job = Lease::take(s, id)?;
    if completed(&job.dir()).is_none_or(|t| t <= now()) {
        return Err(StatusCode::NOT_FOUND);
    }
    let info = key_info(&job.dir()).map_err(|_| INTERNAL)?;
    let mut raw = Zeroizing::new([0; 64]);
    let body = info
        .wrapped
        .strip_prefix(crate::encryption::MAGIC)
        .ok_or(INTERNAL)?;
    crate::encryption::decrypt(
        body,
        info.wrapped.len() as u64,
        &pass,
        Cursor::new(&mut raw[..]),
    )
    .map_err(|_| StatusCode::FORBIDDEN)?;
    if vault::identifier(&raw) != info.identifier {
        return Err(INTERNAL);
    }
    job.unlock(&raw)?;
    let filename = fs::read_to_string(job.data().join("name")).map_err(|_| INTERNAL)?;
    let file = tokio::fs::File::open(job.data().join("result"))
        .await
        .map_err(|_| INTERNAL)?;
    let length = file.metadata().await.map_err(|_| INTERNAL)?.len();
    let mut download = DownloadBody { file, _key: job };
    let stream = async_stream::stream! {
        let mut buf = vec![0;128<<10];
        loop {
            match download.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => yield Ok::<_,io::Error>(axum::body::Bytes::copy_from_slice(&buf[..n])),
                Err(error) => {yield Err(error); break}
            }
        }
    };
    let encoded: String = url::form_urlencoded::byte_serialize(filename.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_LENGTH, length.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename*=UTF-8''{encoded}"),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

// Rust drops fields in declaration order, even if the response is never polled.
struct DownloadBody {
    file: tokio::fs::File,
    _key: Lease,
}
impl DownloadBody {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf).await
    }
}
