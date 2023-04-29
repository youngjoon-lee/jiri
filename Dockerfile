FROM rust:1.69-slim-bullseye AS builder

ARG CARGO_BUILD_ARGS

WORKDIR /usr/src/jiri
COPY . .
RUN cargo build $CARGO_BUILD_ARGS

########################################

FROM debian:bullseye-slim

WORKDIR /usr/local/bin

COPY --from=builder /usr/src/jiri/target/release/jiri .

CMD ["./jiri"]

