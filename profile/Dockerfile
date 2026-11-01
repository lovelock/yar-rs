# Two-stage build for the Rust Yar benchmark image (yar-bench-rust).
# Both binaries live in the image: the server is the default ENTRYPOINT,
# the profiler is selected via `podman run --entrypoint /usr/local/bin/profiler`.
FROM docker.io/library/almalinux:latest AS builder

RUN dnf -y install rust cargo && dnf clean all

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build both benchmark binaries; --locked enforces the Cargo.lock pin.
RUN cargo build --release --locked --bin yar-server-rust --bin profiler

FROM docker.io/library/almalinux:latest

RUN dnf -y install libgcc && dnf clean all

COPY --from=builder /build/target/release/yar-server-rust /usr/local/bin/yar-server-rust
COPY --from=builder /build/target/release/profiler /usr/local/bin/profiler

ENTRYPOINT ["/usr/local/bin/yar-server-rust", "--host", "0.0.0.0:9501"]
