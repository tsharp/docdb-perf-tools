FROM rust:1-bookworm AS builder

WORKDIR /app
COPY . .

RUN make

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --create-home --shell /usr/sbin/nologin benchly

WORKDIR /app
COPY --from=builder /app/scripts/install-pwsh.sh .
COPY --from=builder /app/src/benchly/target/release/benchly /usr/local/bin/benchly

# RUN ./install-pwsh.sh

RUN mkdir -p /app/bench-results \
	&& chown -R benchly:benchly /app

USER benchly
VOLUME ["/app/bench-results"]

ENTRYPOINT ["benchly"]
CMD ["--help"]