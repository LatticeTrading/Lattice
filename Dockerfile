# syntax=docker/dockerfile:1
# Lattice web: `nsh package --target web` builds a static site; Caddy serves it.

FROM debian:trixie-slim AS build
ARG NSH_VERSION=v0.6.1
ARG WASM_BINDGEN_VERSION=0.2.130
# Optional: another Ferris backend (read at compile time).
ARG LATTICE_FERRIS_HTTP=
ARG LATTICE_FERRIS_WS=
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential pkg-config libgtk-4-dev \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/root/.nsh/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
RUN curl -fsSL https://nsh.dev/install.sh | NSH_VERSION=$NSH_VERSION sh
RUN curl -fsSL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$WASM_BINDGEN_VERSION/wasm-bindgen-$WASM_BINDGEN_VERSION-x86_64-unknown-linux-musl.tar.gz" \
      | tar -xz --strip-components=1 -C /usr/local/cargo/bin "wasm-bindgen-$WASM_BINDGEN_VERSION-x86_64-unknown-linux-musl/wasm-bindgen"
WORKDIR /app
COPY rust-toolchain.toml ./
RUN rustup toolchain install
COPY . .
RUN [ -n "$LATTICE_FERRIS_HTTP" ] && export LATTICE_FERRIS_HTTP || unset LATTICE_FERRIS_HTTP; \
    [ -n "$LATTICE_FERRIS_WS" ] && export LATTICE_FERRIS_WS || unset LATTICE_FERRIS_WS; \
    nsh package --target web -o /out

FROM caddy:2-alpine
COPY Caddyfile /etc/caddy/Caddyfile
COPY --from=build /out/Lattice /srv
EXPOSE 80
