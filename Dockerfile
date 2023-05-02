FROM rust:1.69-slim-bullseye AS builder

ARG BUILD_MODE=debug

WORKDIR /usr/src/jiri
COPY . .

WORKDIR /usr/src/jiri/standalone
RUN if [ "$BUILD_MODE" = "release" ]; then cargo build --release; else cargo build; fi

########################################

FROM debian:bullseye-slim

ARG BUILD_MODE=debug

WORKDIR /usr/local/bin

COPY --from=builder /usr/src/jiri/target/$BUILD_MODE/jiri-standalone .

CMD ["./jiri-standalone"]

