FROM rust:1.95-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release --features lan

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/youtube-storage /usr/local/bin/youtube-storage
USER 10001:10001
ENTRYPOINT ["youtube-storage","serve"]
