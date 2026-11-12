# syntax=docker/dockerfile:1

FROM oven/bun:1-slim AS web-builder
WORKDIR /web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web ./
RUN bun run build

FROM rust:1-bookworm AS builder

RUN apt-get update \
	&& apt-get install -y --no-install-recommends cmake clang pkg-config \
	&& rm -rf /var/lib/apt/lists/*

WORKDIR /src

COPY Cargo.toml Cargo.lock build.rs ./
COPY assets ./assets
COPY src ./src
COPY --from=web-builder /web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
	--mount=type=cache,target=/usr/local/cargo/git \
	--mount=type=cache,target=/src/target \
	cargo build --locked --release \
	&& cp /src/target/release/flora /usr/local/bin/flora

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates \
	&& rm -rf /var/lib/apt/lists/* \
	&& groupadd --system --gid 10001 flora \
	&& useradd --system --uid 10001 --gid flora --home-dir /var/lib/flora --shell /usr/sbin/nologin flora \
	&& mkdir -p /var/lib/flora/data /var/lib/flora/buffer /var/lib/flora/storage \
	&& chown -R flora:flora /var/lib/flora

COPY --from=builder /usr/local/bin/flora /usr/local/bin/flora

WORKDIR /var/lib/flora

ENV FLORA_DATA=/var/lib/flora/data \
	FLORA_BUFFER=/var/lib/flora/buffer \
	FLORA_STORAGE=/var/lib/flora/storage \
	FLORA_LISTEN=0.0.0.0:8080 \
	FLORA_LOG=info

USER flora:flora

EXPOSE 8080

VOLUME ["/var/lib/flora/data", "/var/lib/flora/buffer", "/var/lib/flora/storage"]

ENTRYPOINT ["/usr/local/bin/flora"]
CMD ["serve"]
