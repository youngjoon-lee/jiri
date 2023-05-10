FROM rust:1.68 as builder
WORKDIR /usr/src/jiri-rs
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
RUN apt-get update && apt-get install && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/jiri-rs /usr/local/bin/jiri-rs
CMD ["jiri-rs"]
