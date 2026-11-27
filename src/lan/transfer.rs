use axum::body::{Body, Bytes};
use futures_util::StreamExt;
use std::{io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
    time::{timeout_at, Instant},
};

pub(super) fn download_body(
    file: impl AsyncRead + Send + Unpin + 'static,
    lease: impl Send + 'static,
    length: u64,
    deadline: Instant,
    idle: Duration,
) -> Body {
    let (sender, mut receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        // Close the file before evicting its fscrypt key, including on errors.
        let _lease = lease;
        let mut file = file;
        let mut remaining = length;
        while remaining > 0 && Instant::now() < deadline {
            let until = deadline.min(Instant::now() + idle);
            let Ok(Ok(slot)) = timeout_at(until, sender.reserve()).await else {
                break;
            };
            if Instant::now() >= deadline {
                break;
            }
            let mut chunk = vec![0; remaining.min(128 << 10) as usize];
            match file.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    remaining -= n as u64;
                    chunk.truncate(n);
                    slot.send(Bytes::from(chunk));
                }
            }
        }
    });
    Body::from_stream(async_stream::try_stream! {
        let mut remaining = length;
        while remaining > 0 {
            let chunk = receiver.recv().await.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "Download interrupted"))?;
            remaining -= chunk.len() as u64;
            yield chunk;
        }
    }.map(|chunk: io::Result<Bytes>| chunk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tokio::sync::oneshot;

    async fn check_release(idle: Duration, lifetime: Duration) {
        let (lease, mut released) = oneshot::channel::<()>();
        let body = download_body(
            tokio::io::repeat(0),
            lease,
            512 << 10,
            Instant::now() + lifetime,
            idle,
        );
        // Keep the response alive without polling it. Dropping the sender
        // stands in for release of the job's exclusive fscrypt lease.
        assert!(tokio::time::timeout(Duration::from_secs(2), &mut released)
            .await
            .unwrap()
            .is_err());
        assert!(to_bytes(body, 512 << 10).await.is_err());
    }

    #[tokio::test]
    async fn stalled_response_releases_on_inactivity_or_expiry() {
        check_release(Duration::from_millis(20), Duration::from_secs(60)).await;
        check_release(Duration::from_secs(60), Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn active_download_preserves_every_byte() {
        let data = vec![74; 512 << 10];
        let body = download_body(
            io::Cursor::new(data.clone()),
            (),
            data.len() as u64,
            Instant::now() + Duration::from_secs(10),
            Duration::from_secs(5),
        );
        assert_eq!(to_bytes(body, data.len()).await.unwrap().as_ref(), data);
    }
}
