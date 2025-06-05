FROM docker.io/rust:bookworm as builder

WORKDIR /app

COPY . .

RUN cargo build --release

FROM docker.io/debian:bookworm

RUN apt update

RUN apt install -y curl ca-certificates

ENV MUUUXY_SERVER_HOST="0.0.0.0"
ENV MUUUXY_SERVER_PORT=3000
ENV MUUUXY_SERVER_SCHEME="http"
ENV MUUUXY_SERVER_DOMAIN="localhost:3000"

EXPOSE 3000

HEALTHCHECK \
    --start-period=5s \
    --interval=5s \
    --timeout=30s \
    --retries=3 \
    CMD curl --silent --fail http://localhost:3000/healthz || exit 1

COPY --from=builder /app/target/release/muuuxy /usr/local/bin/muuuxy

CMD ["muuuxy"]
