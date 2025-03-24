# Laya: efficient standards-based image API 💅

Laya is a high-performance server implementation of the [IIIF Image API](https://iiif.io/api/image/3.0/) (version 3.0) with an emphasis on safety, performance, and spec-conformance.

### Features

- **IIIF Image API 3.0** compliant implementation
- **Streaming architecture** that efficiently handles large images
- **Flexible storage** with modular backends for local filesystems and S3 object storage

## Status

Laya is currently in active development. Core functionality is working, including:
- JPEG2000 image decoding via Kakadu
- JPEG encoding via MozJPEG
- Multi-threaded processing pipeline
- Filesystem and S3 storage backends
- Tracing and metrics via OpenTelemetry

### Users

Coming soon™

### Developers

#### Requirements:
- Linux
- Rust 1.85 (the project builds on stable, but nightly is used for `rustfmt`).

#### Building:

During development: ```cargo check``` (or ```cargo build``` to produce binaries).

For an optimized build: ```cargo build --release```.

#### Contributing:

Before committing:
```bash
cargo build
cargo clippy --workspace --all-targets --all-features -- -Dwarnings
cargo +nightly fmt --all
```

### Licensing

Laya is dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
