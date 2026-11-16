# Board build environment: the target board runs Ubuntu 20.04 / glibc 2.31.
# Building on a newer host links against a newer glibc (or, with musl, breaks
# dlopen for librockchip_mpp), so the reproducible build runs here.
FROM ubuntu:20.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config \
      gcc-aarch64-linux-gnu libc6-dev-arm64-cross \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal \
 && rustup target add aarch64-unknown-linux-gnu

# Cross linker for the board target (container glibc is 2.31, matching the board).
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

WORKDIR /src
