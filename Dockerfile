FROM rust:1.96-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    libclang-dev \
    clang \
    build-essential \
    perl \
    git \
    && rm -rf /var/lib/apt/lists/*

ENV LIBOPUS_STATIC=1 \
    OPUS_STATIC=1 \
    AUDIOPUS_STATIC=1 \
    CMAKE_POLICY_VERSION_MINIMUM=3.5 \
    CARGO_TERM_COLOR=always

COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --locked --bin kairo

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app
COPY --from=builder --chown=nonroot:nonroot /build/target/release/kairo /app/kairo

ENV KAIRO_CONFIG=/app/application.yml \
    RUST_LOG=info \
    MIMALLOC_PURGE_DELAY=10 \
    MIMALLOC_ARENA_EAGER_COMMIT=0

EXPOSE 2333
ENTRYPOINT ["/app/kairo"]


FROM gcr.io/distroless/cc-debian12:nonroot AS ci
WORKDIR /app
ARG TARGETARCH
COPY bin/linux/${TARGETARCH}/kairo /app/kairo

ENV KAIRO_CONFIG=/app/application.yml \
    RUST_LOG=info \
    MIMALLOC_PURGE_DELAY=10 \
    MIMALLOC_ARENA_EAGER_COMMIT=0

EXPOSE 2333
ENTRYPOINT ["/app/kairo"]
