FROM rust:alpine3.24 AS builder

COPY . .

RUN apk update && apk add musl-dev libressl-dev

RUN cargo build --release -p pesde-registry --features mysql

FROM alpine:3.24

COPY --from=builder /target/release/pesde-registry /usr/local/bin/

CMD ["/usr/local/bin/pesde-registry"]
