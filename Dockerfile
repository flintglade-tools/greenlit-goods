# syntax=docker/dockerfile:1

# Tags are paired with immutable multi-platform manifest digests. Review both
# the Rust version and base-image digest as part of every release.
FROM rust:1.96.1-slim-bookworm@sha256:e18a79fc84dfcfc3ab5ba72290398a644c135c97eaa881447fddc354ee4701a3 AS build

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY engine ./engine
COPY cli ./cli
RUN cargo build --release --locked --package greenlit-cli

FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df

LABEL org.opencontainers.image.title="Greenlit Goods Feed Audit" \
      org.opencontainers.image.description="Offline Google Merchant Center feed preflight for GitHub Actions" \
      org.opencontainers.image.source="https://github.com/flintglade-tools/greenlit-goods" \
      org.opencontainers.image.licenses="MIT"

COPY --from=build /src/target/release/greenlit /usr/local/bin/greenlit
COPY action/entrypoint.sh /usr/local/bin/greenlit-action

ENTRYPOINT ["/usr/local/bin/greenlit-action"]
