# `sekai-exporter` built with the `serve` feature only; see docs/serving.md.

FROM rust:1.99-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p sekai-exporter --no-default-features --features serve \
    && cp target/release/sekai-exporter /usr/local/bin/

FROM debian:bookworm-slim
COPY --from=build /usr/local/bin/sekai-exporter /usr/local/bin/
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["sekai-exporter"]
