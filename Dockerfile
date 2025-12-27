FROM rust:1.86-alpine AS builder

RUN apk add --no-cache musl-dev pkgconf openssl-dev openssl-libs-static

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

ENV OPENSSL_STATIC=1
RUN cargo build --release

FROM alpine:3.21

RUN apk add --no-cache ca-certificates

COPY --from=builder /app/target/release/certstream-server-rust /usr/local/bin/

EXPOSE 8080

CMD ["certstream-server-rust"]
