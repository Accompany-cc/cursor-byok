//! Sends bounded, cancellable HTTP attempts before a provider stream starts.

use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

use super::CallRecorder;

#[derive(Debug)]
pub(crate) enum Attempt {
    Response(reqwest::Response),
    Cancelled,
}

pub(crate) async fn send_with_retry<F>(
    label: &str,
    build: F,
    cancellation: &CancellationToken,
    recorder: Option<&CallRecorder>,
) -> Result<Attempt>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    for delay in [2, 5, 10] {
        let attempt = send_once(label, &build, cancellation, recorder).await;
        let retry = matches!(&attempt, Err(Error::Provider(message)) if message.starts_with(&format!("{label} 429 ")) || message.starts_with(&format!("{label} 502 ")) || message.starts_with(&format!("{label} 503 ")) || message.starts_with(&format!("{label} 504 ")));
        if !retry {
            return attempt;
        }
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(Attempt::Cancelled),
            _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
        }
    }
    send_once(label, build, cancellation, recorder).await
}

pub(crate) async fn send_once<F>(
    label: &str,
    build: F,
    cancellation: &CancellationToken,
    recorder: Option<&CallRecorder>,
) -> Result<Attempt>
where
    F: FnOnce() -> reqwest::RequestBuilder,
{
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Ok(Attempt::Cancelled),
        response = build().send() => response,
    }?;
    if let Some(recorder) = recorder {
        recorder
            .response_headers(response.status().as_u16())
            .await?;
    }
    if response.status().is_success() {
        return Ok(Attempt::Response(response));
    }
    let status = response.status();
    let bytes = tokio::select! {
        _ = cancellation.cancelled() => return Ok(Attempt::Cancelled),
        bytes = response.bytes() => bytes,
    }?;
    Err(Error::Provider(format!(
        "{label} {status}: {}",
        String::from_utf8_lossy(&bytes)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(response: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            socket.write_all(response).await.unwrap();
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn retry_backoff_can_be_cancelled_and_permanent_errors_are_not_retried() {
        let url =
            server(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\ndown").await;
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel.cancel();
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            send_with_retry("test", || client.get(&url), &token, None),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(result, Attempt::Cancelled));
        let url = server(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await;
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(1),
            send_with_retry("test", || client.get(&url), &CancellationToken::new(), None)
        )
        .await
        .unwrap()
        .is_err());
    }

    #[tokio::test]
    async fn non_success_status_is_one_failed_attempt() {
        let url =
            server(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\ndown").await;
        let client = reqwest::Client::new();
        let error = send_once("test", || client.get(&url), &CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Provider(message) if message.contains("503") && message.contains("down"))
        );
    }

    #[tokio::test]
    async fn response_body_transport_failure_is_one_failed_attempt() {
        let url =
            server(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 100\r\n\r\nshort").await;
        let client = reqwest::Client::new();
        let error = send_once("test", || client.get(&url), &CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http(_)));
    }

    #[tokio::test]
    async fn request_transport_failure_is_one_failed_attempt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{address}");
        let client = reqwest::Client::new();
        let error = send_once("test", || client.get(&url), &CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http(_)));
    }
}
