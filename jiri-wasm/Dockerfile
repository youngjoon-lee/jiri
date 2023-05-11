FROM rust:1.68 AS builder

WORKDIR /usr/src/jiri-wasm
COPY . .

WORKDIR /usr/src/jiri-wasm/egui
RUN wget -qO- https://github.com/thedodd/trunk/releases/download/v0.16.0/trunk-x86_64-unknown-linux-gnu.tar.gz | tar -xzf-
RUN rustup target add wasm32-unknown-unknown
RUN ./trunk build --release

FROM node:20-bullseye-slim
RUN apt-get update && apt-get install -y xsel && rm -rf /var/lib/apt/lists/*

RUN npm install --global serve

WORKDIR /data
COPY --from=builder /usr/src/jiri-wasm/egui/dist ./dist

EXPOSE 7070
CMD ["serve", "-p", "7070", "./dist"]
