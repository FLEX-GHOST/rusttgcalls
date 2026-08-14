# Contributing to rusttgcalls

Thank you for your interest in contributing to `rusttgcalls`. We welcome contributions from the community to make this library faster, safer, and more robust.

---

## Code of Conduct

All contributors and participants are expected to adhere to our [Code of Conduct](CODE_OF_CONDUCT.md). Please read it before participating in discussions or submitting pull requests.

---

## Development Workflow

### 1. Prerequisites

- **Rust:** Stable Rust 1.97+ (2024 edition).
- **FFmpeg:** `ffmpeg` on your `PATH` for audio/video transcoding tests.

### 2. Building and Checking

```bash
# Clone the repository
git clone https://github.com/FLEX-GHOST/rusttgcalls.git
cd rusttgcalls

# Check compilation across all targets
cargo check --all-targets

# Run test suite
cargo test

# Live watch during development
cargo watch -w . -w test_runner -x "run --bin test_runner"
```

### 3. Running Performance Benchmarks

Before submitting performance-critical changes, execute the local benchmark suite:

```bash
cd benchmark
cargo run --release
```

---

## Engineering Guidelines

1. **Pure Rust:** Do not introduce C/C++ FFI wrappers or external native dependencies.
2. **Zero Compiler Warnings:** All code must pass `cargo check --all-targets` and `cargo clippy` without warnings.
3. **Memory Safety & Low Footprint:**
   - Avoid unnecessary heap allocations and clones.
   - Use `bytes::Bytes` and `bytes::BytesMut` for zero-copy packet slicing across async boundaries.
   - Use lock-free primitives (`papaya`) and `parking_lot::Mutex` for synchronization.
4. **Async Best Practices:**
   - Never block async tasks with blocking calls or sleeping threads.
   - Always use asynchronous equivalents (`tokio::sync`, `tokio::time`).
5. **No Regressions:** Ensure all existing unit tests, media pacers, and integration examples continue to pass cleanly.

---

## Submitting Pull Requests

1. Fork the repository and create a new feature branch (`git checkout -b feat/my-improvement`).
2. Make focused, well-tested commits with descriptive commit messages.
3. Verify that `cargo check --all-targets` and `cargo test` pass with zero errors and zero warnings.
4. Open a Pull Request on GitHub with a clear explanation of your changes and test results.
