//! Bounded downloads; callers validate platform-owned URLs before entering here.
use crate::Proxy;
use std::time::Duration;

pub async fn bytes(
    proxy: &Proxy,
    url: &str,
    bearer: Option<&str>,
    limit: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let error = || std::io::Error::other("attachment download failed (details redacted)");
    let client = proxy
        .http_builder()
        .map_err(|_| error())?
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| error())?;
    let mut request = client.get(url);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| error())?
        .error_for_status()
        .map_err(|_| error())?;
    if response.status().is_redirection()
        || response.content_length().is_some_and(|n| n > limit as u64)
    {
        return Err(error());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| error())? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(error());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
