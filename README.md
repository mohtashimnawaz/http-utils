# HTTP Client Utilities for Rust

[![Crates.io](https://img.shields.io/crates/v/http-client-utils)](https://crates.io/crates/http-client-utils)
[![Documentation](https://docs.rs/http-client-utils/badge.svg)](https://docs.rs/http-client-utils)
[![License](https://img.shields.io/crates/l/http-client-utils)](LICENSE)
[![CI Status](https://github.com/yourusername/http-client-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/yourusername/http-client-utils/actions)

A lightweight, high-level HTTP client library for Rust with convenient utilities for common operations.

## Features

- Simple API for making HTTP requests (GET, POST, etc.)
- Support for query parameters and headers
- File downloads with progress reporting
- Resumable downloads (supporting Range requests)
- Automatic JSON serialization/deserialization
- Configurable timeouts
- Thread-safe design

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
http-client-utils = "0.1"