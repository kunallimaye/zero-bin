FROM rustlang/rust:nightly-bullseye-slim@sha256:2be4bacfc86e0ec62dfa287949ceb47f9b6d9055536769bdee87b7c1788077a9 as builder

# Install jemalloc
RUN apt-get update && apt-get install -y libjemalloc2 libjemalloc-dev make

RUN \
  mkdir -p common/src  && touch common/src/lib.rs && \
  mkdir -p ops/src     && touch ops/src/lib.rs && \
  mkdir -p worker/src  && echo "fn main() {println!(\"YO!\");}" > worker/src/main.rs

COPY Cargo.toml .
RUN sed -i "2s/.*/members = [\"common\", \"ops\", \"worker\"]/" Cargo.toml
COPY Cargo.lock .

COPY common/Cargo.toml ./common/Cargo.toml
COPY ops/Cargo.toml ./ops/Cargo.toml
COPY worker/Cargo.toml ./worker/Cargo.toml

COPY ./rust-toolchain.toml ./

RUN RUSTFLAGS='-C target-cpu=native' cargo build --release --bin worker 

COPY common ./common
COPY ops ./ops
COPY worker ./worker
RUN \
  touch common/src/lib.rs && \
  touch ops/src/lib.rs && \
  touch worker/src/main.rs

RUN RUSTFLAGS='-C target-cpu=native' cargo build --release --bin worker 

FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ca-certificates libjemalloc2
COPY --from=builder ./target/release/worker /usr/local/bin/worker
CMD ["worker"]
