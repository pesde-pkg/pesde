FROM rust:1.94-alpine3.23 AS builder

COPY . .

RUN apk update && apk add musl-dev libressl-dev

RUN cargo build --release -p pesde-registry

FROM alpine:3.23

COPY --from=builder /target/release/pesde-registry /usr/local/bin/

CMD ["/usr/local/bin/pesde-registry"]