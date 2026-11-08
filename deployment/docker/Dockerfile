# 本番用ではなく、開発と CI で「動くもの」を作るための最小イメージ。
# 依存のビルドを先に済ませるため、マニフェストだけ先にコピーする層を分けている。

FROM rust:1-bookworm AS builder

WORKDIR /src

# 1) 依存だけをビルドする層。ソース変更でここが無効化されない。
COPY Cargo.toml Cargo.lock* ./
COPY crates/domain/Cargo.toml       crates/domain/Cargo.toml
COPY crates/contracts/Cargo.toml    crates/contracts/Cargo.toml
COPY crates/store/Cargo.toml        crates/store/Cargo.toml
COPY crates/providers/Cargo.toml    crates/providers/Cargo.toml
COPY crates/engine-rag/Cargo.toml   crates/engine-rag/Cargo.toml
COPY crates/engine-graph/Cargo.toml crates/engine-graph/Cargo.toml
COPY crates/engine-code/Cargo.toml  crates/engine-code/Cargo.toml
COPY crates/engine-impact/Cargo.toml crates/engine-impact/Cargo.toml
COPY crates/engine-test/Cargo.toml  crates/engine-test/Cargo.toml
COPY crates/runtime/Cargo.toml      crates/runtime/Cargo.toml
COPY crates/app/Cargo.toml          crates/app/Cargo.toml
COPY crates/api/Cargo.toml          crates/api/Cargo.toml
COPY crates/worker/Cargo.toml       crates/worker/Cargo.toml

RUN mkdir -p crates/domain/src crates/contracts/src crates/store/src crates/providers/src \
             crates/engine-rag/src crates/engine-graph/src crates/engine-code/src \
             crates/engine-impact/src crates/engine-test/src crates/runtime/src \
             crates/app/src crates/api/src crates/worker/src \
 && for d in domain contracts store providers engine-rag engine-graph engine-code \
             engine-impact engine-test runtime app; do \
      echo "" > crates/$d/src/lib.rs; \
    done \
 && echo "fn main() {}" > crates/api/src/main.rs \
 && echo "" > crates/api/src/lib.rs \
 && echo "fn main() {}" > crates/worker/src/main.rs \
 && echo "" > crates/worker/src/lib.rs \
 && cargo build --release --bin systivra-api --bin systivra-worker

# 2) 実ソースで差し替えて再ビルド（上の層のキャッシュが効く）
COPY crates ./crates
RUN touch crates/*/src/lib.rs crates/*/src/main.rs \
 && cargo build --release --bin systivra-api --bin systivra-worker

FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends fonts-noto-cjk \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/systivra-api    /usr/local/bin/systivra-api
COPY --from=builder /src/target/release/systivra-worker /usr/local/bin/systivra-worker

RUN mkdir -p /data && chmod 0777 /data
VOLUME /data

USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["systivra-api"]
