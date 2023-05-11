FROM rust:1.68

WORKDIR /usr/src/jiri-wasm
COPY . .

WORKDIR /usr/src/jiri-wasm/egui
RUN wget -qO- https://github.com/thedodd/trunk/releases/download/v0.16.0/trunk-x86_64-unknown-linux-gnu.tar.gz | tar -xzf-
RUN rustup target add wasm32-unknown-unknown
RUN ./trunk build

EXPOSE 7070
CMD ["trunk", "serve", "--port", "7070", "--address", "0.0.0.0"]
