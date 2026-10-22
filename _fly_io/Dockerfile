# Builds the image we ship to fly.io: docs.img, sealed into docs.rti, and the
# othismo that runs it. Build from the repository root:
#
#   docker build -f _fly_io/Dockerfile -t othismo-docs .
#   docker run --rm -p 8080:8080 othismo-docs          # http://localhost:8080
#   fly deploy                                            # uses fly.toml at the root
#
# Fly runs x86_64 machines, so on Apple Silicon add --platform linux/amd64 to
# a local `docker build` that you mean to push.

# 1. Rust: build othismo. Everything after uses this one binary, as the .rti
#    must be built by the same othismo that runs it.
FROM rust:1.95-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
# The runtime embeds its own header from here (src/iface.rs).
COPY as/env.iface as/env.iface
RUN cargo build --release --locked

# 2. Node: compile the site's objects to Wasm and embed their headers, render
#    the docs to HTML, and seal docs.img.
FROM node:20-bookworm-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
COPY as/ as/
RUN npm ci
COPY --from=build /src/target/release/othismo /usr/local/bin/othismo
COPY docs/ docs/
COPY examples/static/ examples/static/
RUN for m in HttpServer StaticSite; do \
      npx asc examples/static/scripts/$m.ts -o examples/static/scripts/$m.wasm --optimize \
      && othismo embed examples/static/scripts/$m.iface examples/static/scripts/$m.wasm || exit 1; \
    done \
 && node examples/static/site.mjs
COPY _fly_io/docs.img /ship/docs.img
RUN mkdir -p /ship/scripts \
 && cp examples/static/scripts/HttpServer.wasm examples/static/scripts/StaticSite.wasm /ship/scripts/ \
 && cp -r examples/static/public /ship/public \
 && othismo build /ship/docs.img -o /ship/docs.rti \
 && othismo --check /ship/docs.rti

# 3. What runs: the binary and the one built image, nothing else.
FROM debian:bookworm-slim
# As PID 1, othismo would ignore SIGTERM (it installs no handlers), so a
# stop would wait out the timeout; tini passes signals on.
RUN apt-get update && apt-get install -y --no-install-recommends tini && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/othismo /usr/local/bin/othismo
COPY --from=web /ship/docs.rti /srv/docs.rti
USER 65534
EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["othismo", "/srv/docs.rti"]
