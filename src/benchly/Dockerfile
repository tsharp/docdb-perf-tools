FROM rust:1-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --create-home --shell /usr/sbin/nologin benchly

WORKDIR /app
COPY --from=builder /app/target/release/benchly /usr/local/bin/benchly

RUN mkdir -p /app/bench-results \
	&& chown -R benchly:benchly /app

USER benchly
VOLUME ["/app/bench-results"]

ENTRYPOINT ["benchly"]
CMD ["--help"]