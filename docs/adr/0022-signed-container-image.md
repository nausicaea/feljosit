# ADR-0022: Publish a signed container image

- Status: Proposed
- Date: 2026-11-27
- Related: ADR-0012, ADR-0018, ADR-0019, ADR-0021

## Context

Some users operate Feljosit on a home server or a NAS. These users want a
container image, not an executable file. The image must have the same security
level as the executables of the GitHub release (ADR-0018):

- The users must be able to verify that the image comes from this repository.
- The image must contain the minimum of software (ADR-0021).
- The image must identify itself with the standard OCI labels.

## Options

1. Do not publish an image. The users make their own image.
2. Compile the image again in the release workflow, on a Rust image, for each platform.
3. Put the static Linux executables of the release into a minimal image, and sign the image.

## Decision

We use option 3:

- The release workflow (`release.yml`) puts the static musl executables of the
  `build` job into the image. The image and the GitHub release thus contain the same,
  tested executables. The multi-platform build (`linux/amd64`, `linux/arm64`) has
  no `RUN` instruction. It needs no emulation (QEMU).
- The base image is `gcr.io/distroless/static-debian12:nonroot`. It contains no
  shell and no package manager. It contains only the CA certificates, the time zone
  data, `/tmp` and the user `nonroot` (UID 65532). The application runs as this user.
- Without a build argument, the `Dockerfile` compiles Feljosit from the source code
  on `rust:VERSION-alpine`. CI does this build for each pull request and tests the image.
- The workflow publishes the image to `ghcr.io/nausicaea/feljosit`, with the tags
  `VERSION`, `MAJOR.MINOR` and `latest`.
- `docker/metadata-action` writes the OCI labels (`org.opencontainers.image.*`) on
  each image and the OCI annotations on the image index. The `Dockerfile` also has
  the static labels, for a local build.
- BuildKit attaches a SLSA provenance (`mode=max`) and an SBOM (SPDX) to the image.
- Cosign signs the image digest without a key (Sigstore keyless). The certificate
  identifies the workflow `release.yml` and the tag. Then the workflow verifies the
  signature. The signature is in the public Rekor transparency log.
- All base images and actions are pinned to a digest (ADR-0012). Renovate updates them.
- The image sets `FELJOSIT_HOST=0.0.0.0`, because the port of the container must be
  accessible from the host. The documentation tells the users to publish the port
  only on `127.0.0.1` of the host.

## Consequences

- Good: The users can verify the signature, the provenance and the SBOM with `cosign`.
- Good: The image is small (approximately 3.5 MB compressed) and has no shell.
- Good: The release workflow compiles each executable only one time.
- Bad: In a container, the connection from the host comes from the bridge address,
  not from a loopback address. Thus the first user cannot be added in the browser
  (ADR-0019). The user must use `docker exec -it NAME feljosit create-user USER`.
- Bad: The process has PID 1 and does not stop on `SIGTERM`. The users must start
  the container with `--init` (or `init: true` in Compose) for a fast stop.
- Bad: The keyless signature needs the public Sigstore services (Fulcio and Rekor).
