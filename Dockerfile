FROM rust:1.75 as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/tokenhub .

RUN mkdir -p /app/data

EXPOSE 8080

ENV RUST_LOG=info
ENV CONFIG_PATH=/app/data/config.toml

CMD ["./tokenhub"]
