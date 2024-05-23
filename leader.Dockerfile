FROM rustlang/rust:nightly-bullseye-slim@sha256:2be4bacfc86e0ec62dfa287949ceb47f9b6d9055536769bdee87b7c1788077a9 as builder

# Install jemalloc
RUN apt-get update && apt-get install -y libjemalloc2 libjemalloc-dev make

RUN \
  mkdir -p ops/src     && touch ops/src/lib.rs && \
  mkdir -p common/src  && touch common/src/lib.rs && \
  mkdir -p rpc/src     && touch rpc/src/lib.rs && \
  mkdir -p prover/src  && touch prover/src/lib.rs && \
  mkdir -p leader/src  && echo "fn main() {println!(\"YO!\");}" > leader/src/main.rs

COPY Cargo.toml .
RUN sed -i "2s/.*/members = [\"ops\", \"leader\", \"common\", \"rpc\", \"prover\"]/" Cargo.toml
COPY Cargo.lock .

COPY ops/Cargo.toml ./ops/Cargo.toml
COPY common/Cargo.toml ./common/Cargo.toml
COPY rpc/Cargo.toml ./rpc/Cargo.toml
COPY prover/Cargo.toml ./prover/Cargo.toml
COPY leader/Cargo.toml ./leader/Cargo.toml

COPY rust-toolchain.toml .
COPY .cargo ./.cargo

COPY ops ./ops
COPY common ./common
COPY rpc ./rpc
COPY prover ./prover
COPY leader ./leader
RUN \
  touch ops/src/lib.rs && \
  touch common/src/lib.rs && \
  touch rpc/src/lib.rs && \
  touch prover/src/lib.rs && \
  touch leader/src/main.rs

RUN cargo build --release --bin leader

FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ca-certificates libjemalloc2
COPY --from=builder ./target/release/leader /usr/local/bin/leader
CMD ["leader"]
