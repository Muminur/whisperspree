//! Guarded outbound HTTP construction.
//!
//! Keeping the client constructor here makes it auditable that application
//! network traffic enters only through an explicit feature-owned request.

#[derive(Debug, Clone)]
pub struct NetworkGuard {
    client: reqwest::Client,
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkGuardError {
    #[error("HTTP client construction failed: {0}")]
    Client(#[from] reqwest::Error),
    #[error("outbound URL must be credential-free HTTPS")]
    UnsafeUrl,
}

impl NetworkGuard {
    // This is the single audited reqwest construction point. Call sites must use
    // `NetworkGuard::request`, so the crate-wide lint remains enforced elsewhere.
    #[allow(clippy::disallowed_methods)]
    pub fn new() -> Result<Self, NetworkGuardError> {
        Ok(Self {
            client: reqwest::Client::builder().build()?,
        })
    }

    pub fn request(&self, url: &str) -> Result<reqwest::RequestBuilder, NetworkGuardError> {
        self.request_method(url, reqwest::Method::GET)
    }

    pub fn request_method(
        &self,
        url: &str,
        method: reqwest::Method,
    ) -> Result<reqwest::RequestBuilder, NetworkGuardError> {
        let url = reqwest::Url::parse(url).map_err(|_| NetworkGuardError::UnsafeUrl)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(NetworkGuardError::UnsafeUrl);
        }
        Ok(self.client.request(method, url))
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkGuard;

    #[test]
    fn construction_does_not_send_a_request() {
        NetworkGuard::new().expect("client construction must not perform network I/O");
    }

    #[test]
    fn request_rejects_insecure_or_credentialed_urls_before_network_io() {
        let guard = NetworkGuard::new().unwrap();
        for url in [
            "http://models.example.invalid/model.bin",
            "https://user:pass@models.example.invalid/model.bin",
        ] {
            assert!(guard.request(url).is_err(), "must reject {url}");
        }
    }
}
