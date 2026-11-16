# syntax=docker/dockerfile:1
# openm2's own server (docs/server.md): `docker compose up` runs
# it with Postgres. Base images are pinned by digest; update them on purpose.

FROM rust:1.99.1-slim-bookworm@sha256:a24e292b4af7bdee351a31d3a6379972a5652f0ea22a1b6d6883324ac4f5097c AS build
WORKDIR /src
# The image's toolchain is the pinned one; rust-toolchain.toml's clippy and
# rustfmt are not needed to build.
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
COPY . .
# aws-lc-sys (TLS) builds its C code with the image's gcc.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --profile dist --locked --package m2-server --bin openm2-server \
    && cp target/dist/openm2-server /openm2-server \
    && mkdir /openm2-data

# A small runtime: glibc and libgcc, no shell, a non-root user.
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
COPY --from=build /openm2-server /usr/local/bin/openm2-server
# The terms that must travel with data built into the binary: webpki-roots'
# list of authorities, CDLA-Permissive-2.0.
COPY deploy/licenses/ /usr/share/doc/openm2-server/licenses/
# The server's folder (its self-signed certificate), owned by the non-root
# user; a volume mounted there starts with that owner.
COPY --from=build --chown=65532:65532 /openm2-data /data
# Inside the container the server listens on every address; compose.yaml
# publishes the ports on the host's 127.0.0.1 unless told otherwise.
ENV OPENM2_LISTEN=0.0.0.0 OPENM2_DATA=/data
EXPOSE 11002 13001
USER nonroot
ENTRYPOINT ["/usr/local/bin/openm2-server"]
CMD ["serve"]
