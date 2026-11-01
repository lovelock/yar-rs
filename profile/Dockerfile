# Two-stage build for the Rust Yar benchmark image (yar-bench-rust).
# Both binaries live in the image: the server is the default ENTRYPOINT,
# the profiler is selected via `podman run --entrypoint /usr/local/bin/profiler`.
FROM docker.io/library/almalinux:latest AS builder

# System repos -> Aliyun (upstream mirrorlist default would leave CN slow/blocked).
RUN sed -i \
      -e 's|^mirrorlist=|#mirrorlist=|g' \
      -e 's|^# baseurl=https://repo.almalinux.org|baseurl=https://mirrors.aliyun.com|g' \
      -e 's|^# baseurl=https://vault.almalinux.org|baseurl=https://mirrors.aliyun.com/almalinux|g' \
      /etc/yum.repos.d/almalinux*.repo \
    && dnf -q makecache && dnf clean all

RUN dnf -y install rust cargo && dnf clean all

# Cargo deps -> USTC sparse index (mirrors must be baked into the image,
# never rely on the host's proxy/mirror config).
ENV CARGO_HOME=/root/.cargo
RUN mkdir -p "$CARGO_HOME" && \
    printf '%s\n' \
      '[source.crates-io]' \
      'replace-with = "ustc"' \
      '' \
      '[source.ustc]' \
      'registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"' \
      > "$CARGO_HOME/config.toml"

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build both benchmark binaries; --locked enforces the Cargo.lock pin.
RUN cargo build --release --locked --bin yar-server-rust --bin profiler

FROM docker.io/library/almalinux:latest

# System repos -> Aliyun (kept consistent with the builder stage).
RUN sed -i \
      -e 's|^mirrorlist=|#mirrorlist=|g' \
      -e 's|^# baseurl=https://repo.almalinux.org|baseurl=https://mirrors.aliyun.com|g' \
      -e 's|^# baseurl=https://vault.almalinux.org|baseurl=https://mirrors.aliyun.com/almalinux|g' \
      /etc/yum.repos.d/almalinux*.repo \
    && dnf -q makecache && dnf clean all

RUN dnf -y install libgcc && dnf clean all

COPY --from=builder /build/target/release/yar-server-rust /usr/local/bin/yar-server-rust
COPY --from=builder /build/target/release/profiler /usr/local/bin/profiler

ENTRYPOINT ["/usr/local/bin/yar-server-rust", "--host", "0.0.0.0:9501"]
