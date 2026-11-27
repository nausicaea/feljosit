# syntax=docker/dockerfile:1@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183
#
# The container image of Feljosit (see ADR-0022).
#
# Build from the source code:
#   docker build -t feljosit .
#
# The release workflow uses the executables that it built and tested before:
#   docker buildx build --build-arg BINARY=prebuilt --platform linux/amd64,linux/arm64 .
# Then the files dist/linux-amd64/feljosit and dist/linux-arm64/feljosit must exist.

# "source" or "prebuilt".
ARG BINARY=source

# Stage: compile a static executable (musl) from the source code.
FROM docker.io/library/rust:1.100.0-alpine@sha256:8d0e82abfdf9a38071e07cabc5afba4ddbe077631a80e4e742a760c38beadc33 AS source
# The build compiles SQLite. It needs the C library headers of musl.
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY static ./static
COPY templates ./templates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked \
    && install -D -m 0755 target/release/feljosit /out/feljosit \
    && /out/feljosit --version

# Stage: an executable that the release workflow built before.
FROM scratch AS prebuilt
ARG TARGETOS
ARG TARGETARCH
COPY --chmod=0755 dist/${TARGETOS}-${TARGETARCH}/feljosit /out/feljosit

# Stage: the selected executable.
FROM ${BINARY} AS binary

# Stage: the runtime image. The static distroless image contains only the CA
# certificates, the time zone data, /tmp and the user "nonroot" (UID 65532).
FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="Feljosit" \
      org.opencontainers.image.description="Local-first personal budgeting for the Swiss financial sector, with ISO 20022 camt.053 import." \
      org.opencontainers.image.url="https://github.com/nausicaea/feljosit" \
      org.opencontainers.image.source="https://github.com/nausicaea/feljosit" \
      org.opencontainers.image.documentation="https://github.com/nausicaea/feljosit/blob/main/README.md" \
      org.opencontainers.image.licenses="Unlicense" \
      org.opencontainers.image.vendor="nausicaea" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.base.name="gcr.io/distroless/static-debian12:nonroot" \
      org.opencontainers.image.base.digest="sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab"
COPY --from=binary /out/feljosit /usr/local/bin/feljosit
# The container must listen on all interfaces of its network namespace. Publish
# the port only on the loopback address of the host (see README.md).
ENV FELJOSIT_DATA_DIR=/data \
    FELJOSIT_HOST=0.0.0.0 \
    FELJOSIT_PORT=8765
USER 65532:65532
# BuildKit makes the directory for the user of the USER instruction.
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8765
ENTRYPOINT ["/usr/local/bin/feljosit"]
CMD ["serve"]
