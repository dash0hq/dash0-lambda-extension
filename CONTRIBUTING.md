
## 🏗️ Building the package

This package uses  *Make* to build binary executables for *aarch64* and *x86_64* using `cargo cross`.

**Targets**: `x86_64-unknown-linux-musl` for x86_64 Lambdas and `aarch64-unknown-linux-musl` for Gravaton-based Lambdas (64-bit ARM) with **musl** libc. Building statically with musl keeps the binary compatible with both Amazon Linux 2 runtimes (Python 3.9–3.11) and Amazon Linux 2023 runtimes (Python 3.12+) so you avoid `GLIBC_2.x` errors at cold start.

### Dependencies 

* Make tools
* Rust standard: https://www.rust-lang.org/tools/install (tested with 1.72)
* Cargo cross (~0.2.5)
  * cross-compiling to different targets; Docker or podman based
  * Install with `cargo install cross`
  * Example usage: `cross build --release --target x86_64-unknown-linux-musl`
* `zip` tool
* `aws` CLI tool 
* `jq` JSON manipulation tool

### Building and Deploying the Extension Layer

With the dependencies installed, run `make` in the base directory 
to compile the binaries.  The executables and  are copied into `build/`. The extension entrypoint script (`opt/entrypoint`) is copied to `build/extensions/dash0`
