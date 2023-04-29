FROM rust:1.69-slim-bullseye AS builder

WORKDIR /usr/src/jiri
COPY . .
RUN cargo build --release

########################################

FROM debian:bullseye-slim

WORKDIR /usr/local/bin

COPY --from=builder /usr/src/jiri/target/release/jiri .

CMD ["./jiri"]

