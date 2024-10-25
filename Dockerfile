FROM rust:1.82-bookworm AS builder

COPY . .

RUN cargo build --release -p pesde-registry

FROM debian:bookworm-slim

COPY --from=builder /target/release/pesde-registry /usr/local/bin/

RUN apt-get update && apt-get install -y ca-certificates

CMD ["/usr/local/bin/pesde-registry"]
