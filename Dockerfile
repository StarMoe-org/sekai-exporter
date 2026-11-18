# Two images of `sekai-exporter`. The default target holds the command built with the `serve`
# feature only; see docs/serving.md. The target `full` holds it built with every feature, with
# ffmpeg for the encodings that need it, to update regions and publish them; see
# docs/publishing.md.

FROM rust:1.99-slim-bookworm AS build-full
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=cache,id=registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=target-full,target=/src/target \
    cargo build --locked --release -p sekai-exporter --features full \
    && cp target/release/sekai-exporter /usr/local/bin/

FROM debian:bookworm-slim AS full
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build-full /usr/local/bin/sekai-exporter /usr/local/bin/
USER 65532:65532
ENTRYPOINT ["sekai-exporter"]

FROM rust:1.99-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,id=registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=target-serve,target=/src/target \
    cargo build --locked --release -p sekai-exporter --no-default-features --features serve \
    && cp target/release/sekai-exporter /usr/local/bin/

FROM debian:bookworm-slim
COPY --from=build /usr/local/bin/sekai-exporter /usr/local/bin/
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["sekai-exporter"]
