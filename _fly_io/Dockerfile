# Builds the image we ship to fly.io: docs.img, sealed into docs.rti, and the
# othismo that runs it. Build from the repository root:
#
#   docker build -f _fly_io/Dockerfile -t othismo-docs .
#   docker run --rm -p 8080:8080 othismo-docs          # http://localhost:8080
#   fly deploy                                            # uses fly.toml at the root
#
# Fly runs x86_64 machines, so on Apple Silicon add --platform linux/amd64 to
# a local `docker build` that you mean to push.

# 1. Node: compile the site's objects to Wasm, and render the docs to HTML.
FROM node:20-bookworm-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
COPY as/ as/
RUN npm ci
COPY docs/ docs/
COPY examples/static/ examples/static/
RUN npx asc examples/static/scripts/HttpServer.ts -o examples/static/scripts/HttpServer.wasm --optimize \
 && npx asc examples/static/scripts/StaticSite.ts -o examples/static/scripts/StaticSite.wasm --optimize \
 && node examples/static/site.mjs

# 2. Rust: build othismo, then use it to seal docs.img. The .rti must be
#    built by the same othismo that runs it.
FROM rust:1.95-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
# The checker embeds the runtime's message types from here (src/check.rs).
COPY as/env.ts as/env.ts
RUN cargo build --release --locked
COPY _fly_io/docs.img /ship/docs.img
COPY --from=web /src/examples/static/scripts/HttpServer.wasm /src/examples/static/scripts/StaticSite.wasm /ship/scripts/
COPY --from=web /src/examples/static/public/ /ship/public/
RUN target/release/othismo build /ship/docs.img -o /ship/docs.rti \
 && target/release/othismo --check /ship/docs.rti

# 3. What runs: the binary and the one built image, nothing else.
FROM debian:bookworm-slim
# As PID 1, othismo would ignore SIGTERM (it installs no handlers), so a
# stop would wait out the timeout; tini passes signals on.
RUN apt-get update && apt-get install -y --no-install-recommends tini && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/othismo /usr/local/bin/othismo
COPY --from=build /ship/docs.rti /srv/docs.rti
USER 65534
EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["othismo", "/srv/docs.rti"]
