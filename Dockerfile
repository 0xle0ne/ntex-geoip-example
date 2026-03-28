FROM rust:1.94.0-bookworm AS builder

WORKDIR /app

COPY Cargo.toml ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN groupadd --system app && useradd --system --gid app --create-home --home-dir /app app

WORKDIR /app

COPY --from=builder /app/target/release/geo-api /usr/local/bin/geo-api
COPY mmdb ./mmdb

EXPOSE 8585

USER app

CMD ["/usr/local/bin/geo-api"]