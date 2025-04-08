//! # HTTP Client Utilities
//!
//! A convenient HTTP client library for Rust with support for:
//! - Simple API for making HTTP requests (GET, POST, etc.)
//! - Query parameters and headers support
//! - File downloads with progress reporting
//! - Resumable downloads
//! - JSON serialization/deserialization
//!
//! ## Examples
//!
//! ```no_run
//! use http_client_utils::HttpClient;
//! use std::path::Path;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Simple GET request
//!     let client = HttpClient::new();
//!     let response = client.get("https://httpbin.org/get").await.unwrap();
//!     println!("Status: {}", response.status());
//!
//!     // File download with progress
//!     client.download_file(
//!         "https://example.com/file.zip",
//!         Path::new("file.zip"),
//!         |downloaded, total| {
//!             println!("Downloaded: {}/{} bytes", downloaded, total);
//!         }
//!     ).await.unwrap();
//!
//!     // Resumable download
//!     client.download_file_with_resume(
//!         "https://example.com/large-file.zip",
//!         Path::new("large-file.zip"),
//!         |downloaded, total| {
//!             let percent = (downloaded as f64 / total as f64) * 100.0;
//!             println!("Progress: {:.1}%", percent);
//!         }
//!     ).await.unwrap();
//! }
//! ```

use std::path::Path;
use std::time::Duration;
use reqwest::{Client, Response, header};
use futures::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
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
    use tokio::fs;

    #[tokio::test]
    async fn test_get_request() {
        // This would be better tested with a mock server in integration tests
        // For now, we'll just test against httpbin
        let client = HttpClient::new();
        let response = client.get("https://httpbin.org/get").await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn test_file_download() {
        let temp_dir = std::env::temp_dir();
        let dest = temp_dir.join("test_download.txt");
        
        // Clean up if file exists
        let _ = fs::remove_file(&dest).await;
        
        let client = HttpClient::new();
        let test_content = "test file content";
        
        // Create a temporary HTTP server would be better, but for simplicity:
        // Note: In a real project, you should use mockito for this
        let mut progress_values = Vec::new();
        let result = client.download_file(
            "https://httpbin.org/bytes/16", // Small test file
            &dest,
            |downloaded, total| {
                progress_values.push((downloaded, total));
            }
        ).await;
        
        assert!(result.is_ok());
        assert!(!progress_values.is_empty());
        
        // Clean up
        let _ = fs::remove_file(&dest).await;
    }
}