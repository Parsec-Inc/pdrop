FROM docker.io/library/rust:1-alpine AS build
COPY src/bin/pdrop-relay.rs /src/pdrop-relay.rs
RUN rustc --edition 2024 --crate-name pdrop_relay -C opt-level=3 -C strip=symbols \
    -C target-feature=+crt-static -o /pdrop-relay /src/pdrop-relay.rs

FROM scratch
COPY --from=build /pdrop-relay /pdrop-relay
USER 65534:65534
EXPOSE 4443/tcp
STOPSIGNAL SIGKILL
ENTRYPOINT ["/pdrop-relay"]
CMD ["--listen", "0.0.0.0:4443"]
