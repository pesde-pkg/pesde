FROM rust:1.87-alpine3.21 AS builder

COPY . .

RUN apk update && apk add musl-dev libressl-dev

RUN cargo build --release -p pesde-registry

FROM alpine:3.21

COPY --from=builder /target/release/pesde-registry /usr/local/bin/

CMD ["/usr/local/bin/pesde-registry"]
