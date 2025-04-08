

use std::path::Path;
use std::time::Duration;
use reqwest::{Client, Response, RequestBuilder, Body, header};
use futures::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::io::StreamReader;
use bytes::Bytes;
use thiserror::Error;
use serde::{Serialize, de::DeserializeOwned};

/// Error type for HTTP client operations
#[derive(Error, Debug)]
pub enum HttpClientError {
    #[error("Request error: {0}")]
    RequestError(#[from] reqwest::Error),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Invalid URL: {0}")]
    UrlError(String),
    
    #[error("Timeout reached")]
    TimeoutError,
    
    #[error("Download failed: {0}")]
    DownloadError(String),
    
    #[error("Resume not supported by server")]
    ResumeNotSupported,
}

/// HTTP client with various utility methods
#[derive(Debug, Clone)]
pub struct HttpClient {
    client: Client,
    base_url: Option<String>,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    /// Create a new HTTP client with default settings
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: None,
        }
    }

    /// Create a new HTTP client with a base URL
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: Some(base_url.into()),
        }
    }

    /// Create a new HTTP client with a custom reqwest Client
    pub fn with_client(client: Client) -> Self {
        Self {
            client,
            base_url: None,
        }
    }

    /// Set a timeout for all requests
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to build client with timeout");
        self
    }

    fn build_url(&self, endpoint: &str) -> Result<String, HttpClientError> {
        match &self.base_url {
            Some(base) => Ok(format!("{}{}", base, endpoint)),
            None => Ok(endpoint.to_string()),
        }
    }

    /// Send a GET request
    pub async fn get(&self, endpoint: &str) -> Result<Response, HttpClientError> {
        let url = self.build_url(endpoint)?;
        self.client.get(&url)
            .send()
            .await
            .map_err(Into::into)
    }

    /// Send a GET request with query parameters
    pub async fn get_with_query<T: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        query: &T,
    ) -> Result<Response, HttpClientError> {
        let url = self.build_url(endpoint)?;
        self.client.get(&url)
            .query(query)
            .send()
            .await
            .map_err(Into::into)
    }

    /// Send a POST request with JSON body
    pub async fn post<T: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        body: &T,
    ) -> Result<Response, HttpClientError> {
        let url = self.build_url(endpoint)?;
        self.client.post(&url)
            .json(body)
            .send()
            .await
            .map_err(Into::into)
    }

    /// Send a POST request with raw bytes
    pub async fn post_raw(
        &self,
        endpoint: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<Response, HttpClientError> {
        let url = self.build_url(endpoint)?;
        self.client.post(&url)
            .header("Content-Type", content_type)
            .body(body)
            .send()
            .await
            .map_err(Into::into)
    }

    /// Download a file with progress reporting
    pub async fn download_file(
        &self,
        url: &str,
        destination: &Path,
        progress_callback: impl Fn(u64, u64),
    ) -> Result<(), HttpClientError> {
        let response = self.client.get(url)
            .send()
            .await?;

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut file = File::create(destination).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            progress_callback(downloaded, total_size);
        }

        Ok(())
    }

    /// Download a file with resume support and progress reporting
    pub async fn download_file_with_resume(
        &self,
        url: &str,
        destination: &Path,
        progress_callback: impl Fn(u64, u64),
    ) -> Result<(), HttpClientError> {
        // Try to open the file in append mode if it exists
        let file_exists = destination.exists();
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination)
            .await?;

        let mut downloaded_bytes = if file_exists {
            file.metadata().await?.len()
        } else {
            0
        };

        // Create a request builder with Range header if resuming
        let mut request = self.client.get(url);
        if downloaded_bytes > 0 {
            request = request.header(header::RANGE, format!("bytes={}-", downloaded_bytes));
        }

        // Send the request
        let response = request.send().await?;

        // Check response status
        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(HttpClientError::DownloadError(format!(
                "Server returned error status: {}", status
            )));
        }

        // Handle server that doesn't support resume
        if downloaded_bytes > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(HttpClientError::ResumeNotSupported);
        }

        // Get total content length
        let total_size = match status {
            reqwest::StatusCode::PARTIAL_CONTENT => {
                // Parse Content-Range header for partial content
                response.headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| {
                        s.split('/').last().and_then(|s| s.parse::<u64>().ok())
                    })
                    .unwrap_or(downloaded_bytes + response.content_length().unwrap_or(0))
            }
            _ => {
                downloaded_bytes + response.content_length().unwrap_or(0)
            }
        };

        // Download chunks
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded_bytes += chunk.len() as u64;
            progress_callback(downloaded_bytes, total_size);
        }

        Ok(())
    }

    /// Parse response as JSON
    pub async fn json<T: DeserializeOwned>(response: Response) -> Result<T, HttpClientError> {
        response.json::<T>().await.map_err(Into::into)
    }

    /// Get response as text
    pub async fn text(response: Response) -> Result<String, HttpClientError> {
        response.text().await.map_err(Into::into)
    }

    /// Get response as bytes
    pub async fn bytes(response: Response) -> Result<Bytes, HttpClientError> {
        response.bytes().await.map_err(Into::into)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{mock, Server};
    use tempfile::NamedTempFile;
    use serde_json::json;

    #[tokio::test]
    async fn test_get_request() {
        let mut server = Server::new();
        let _m = mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("hello world")
            .create();

        let client = HttpClient::with_base_url(server.url());
        let response = client.get("/test").await.unwrap();
        assert_eq!(response.status(), 200);
        let body = response.text().await.unwrap();
        assert_eq!(body, "hello world");
    }

    #[tokio::test]
    async fn test_file_download() {
        let mut server = Server::new();
        let _m = mock("GET", "/file")
            .with_status(200)
            .with_header("content-type", "application/octet-stream")
            .with_body("file content")
            .create();

        let dest = NamedTempFile::new().unwrap();
        let client = HttpClient::new();
        
        let mut progress_values = Vec::new();
        client.download_file(
            &format!("{}/file", server.url()),
            dest.path(),
            |downloaded, total| {
                progress_values.push((downloaded, total));
            }
        ).await.unwrap();

        assert!(!progress_values.is_empty());
        let content = std::fs::read_to_string(dest.path()).unwrap();
        assert_eq!(content, "file content");
    }

    #[tokio::test]
    async fn test_resumable_download() {
        let mut server = Server::new();
        let _m = mock("GET", "/large-file")
            .with_status(206)
            .with_header("content-type", "application/octet-stream")
            .with_header("content-range", "bytes 10-19/20")
            .with_body("continued")
            .create_async()
            .await;

        let dest = NamedTempFile::new().unwrap();
        // Create a file with partial content
        tokio::fs::write(dest.path(), "existing d").await.unwrap();

        let client = HttpClient::new();
        
        let mut progress_values = Vec::new();
        client.download_file_with_resume(
            &format!("{}/large-file", server.url()),
            dest.path(),
            |downloaded, total| {
                progress_values.push((downloaded, total));
            }
        ).await.unwrap();

        assert!(!progress_values.is_empty());
        let content = std::fs::read_to_string(dest.path()).unwrap();
        assert_eq!(content, "existing dcontinued");
    }
}
