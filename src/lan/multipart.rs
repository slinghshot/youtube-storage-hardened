use super::{Web, INVALID};
use axum::{
    body::{Body, Bytes},
    extract::{multipart::Field as AxumField, FromRequest, Multipart, Request},
    http::StatusCode,
    RequestExt,
};
use futures_util::StreamExt;
use std::{
    future::{poll_fn, Future},
    io,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
    time::Duration,
};

// Limit read-ahead at the parser input. Multer can poll a ready input stream
// repeatedly before it examines its buffer for the boundary or header terminator.
const SLICE: usize = 16 * 1024;
const HEADER_BUDGET: usize = 2 * SLICE;
// Give the parser one slice only after it has tried to parse its existing
// buffer. Otherwise successive short fields can accumulate speculative input.
#[derive(Default)]
struct Demand {
    granted: AtomicBool,
    requested: AtomicBool,
}
impl Demand {
    async fn wait(&self) {
        poll_fn(|_| {
            if self.granted.swap(false, Ordering::Relaxed) {
                Poll::Ready(())
            } else {
                self.requested.store(true, Ordering::Relaxed);
                Poll::Pending
            }
        })
        .await
    }
    async fn parse<T>(&self, future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        self.granted.store(false, Ordering::Relaxed);
        poll_fn(|cx| {
            self.requested.store(false, Ordering::Relaxed);
            let result = future.as_mut().poll(cx);
            if result.is_pending() && self.requested.load(Ordering::Relaxed) {
                self.granted.store(true, Ordering::Relaxed);
                cx.waker().wake_by_ref();
            }
            result
        })
        .await
    }
}
pub(super) struct UploadField<'a> {
    field: AxumField<'a>,
    demand: &'a Demand,
}
impl UploadField<'_> {
    pub(super) fn name(&self) -> Option<&str> {
        self.field.name()
    }
    pub(super) async fn chunk(
        &mut self,
    ) -> Result<Option<Bytes>, axum::extract::multipart::MultipartError> {
        self.demand.parse(self.field.chunk()).await
    }
}
pub(super) struct UploadParts {
    parser: Multipart,
    demand: Arc<Demand>,
    remaining: Arc<AtomicUsize>,
}
impl UploadParts {
    pub(super) async fn new(request: Request) -> Web<Self> {
        let (head, body) = request.with_limited_body().into_parts();
        let remaining = Arc::new(AtomicUsize::new(HEADER_BUDGET));
        let budget = remaining.clone();
        let demand = Arc::new(Demand::default());
        let reader = demand.clone();
        let stream = async_stream::try_stream! {
            let mut input = body.into_data_stream();
            while let Some(frame) = input.next().await {
                let mut frame = frame.map_err(io::Error::other)?;
                while !frame.is_empty() {
                    reader.wait().await;
                    let size = SLICE.min(frame.len());
                    let left = budget.load(Ordering::Relaxed);
                    if left != usize::MAX {
                        let rest = left.checked_sub(size).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Multipart framing exceeds limit"))?;
                        budget.store(rest, Ordering::Relaxed);
                    }
                    yield frame.split_to(size);

                }
            }
        };
        let body = Body::from_stream(stream.map(|chunk: io::Result<_>| chunk));
        let parser = Multipart::from_request(Request::from_parts(head, body), &())
            .await
            .map_err(|_| INVALID)?;
        Ok(Self {
            parser,
            demand,
            remaining,
        })
    }
    pub(super) async fn next_field(&mut self) -> Web<Option<UploadField<'_>>> {
        // The previous field can leave at most one slice plus a boundary in
        // the parser. Only payload reads may consume an unlimited byte count.
        self.remaining.store(HEADER_BUDGET, Ordering::Relaxed);
        let result = tokio::time::timeout(
            Duration::from_secs(90),
            self.demand.parse(self.parser.next_field()),
        )
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| INVALID)?;
        self.remaining.store(usize::MAX, Ordering::Relaxed);
        Ok(result.map(|field| UploadField {
            field,
            demand: &self.demand,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parts(header: &str, payload: &[u8]) -> UploadParts {
        let mut data = format!(
            "--sample\r\nContent-Disposition: form-data; name=\"file\"\r\nX-Note: {header}\r\n\r\n"
        )
        .into_bytes();
        data.extend_from_slice(payload);
        data.extend_from_slice(b"\r\n--sample--\r\n");
        UploadParts::new(
            Request::post("/jobs")
                .header("content-type", "multipart/form-data; boundary=sample")
                .body(Body::from(data))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn framing_budget_is_independent_of_payload_size() {
        let payload = vec![42; 12 * SLICE];
        let mut parts = parts("short", &payload).await;
        let mut field = parts.next_field().await.unwrap().unwrap();
        let mut received = Vec::new();
        while let Some(chunk) = field.chunk().await.unwrap() {
            received.extend_from_slice(&chunk);
        }
        assert_eq!(received, payload);
        drop(field);
        assert!(parts.next_field().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn oversized_metadata_is_rejected() {
        let mut parts = parts(&"a".repeat(HEADER_BUDGET * 2), b"").await;
        assert!(matches!(parts.next_field().await, Err(INVALID)));
    }
}
