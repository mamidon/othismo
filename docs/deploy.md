# Deploying the docs site image

CI builds the container defined by [`_fly_io/Dockerfile`](../_fly_io/Dockerfile)
and, on `master` and on semver tags (`vX.Y.Z`), publishes it to GHCR:

`ghcr.io/mamidon/othismo`

## Image tags

| Tag | When |
|---|---|
| `master` | Moving tip of the default branch |
| `sha-<short>` | Immutable commit (every successful publish) |
| `1.2.3`, `1.2`, `1` | From a git tag `v1.2.3` |

Pull requests run the same Dockerfile build but **do not** push to GHCR.

After the first successful publish you may need to set the package visibility
to Public under the repo’s Packages settings so unauthenticated pulls work.

## Run locally

```sh
docker pull ghcr.io/mamidon/othismo:master
docker run --rm -p 8080:8080 ghcr.io/mamidon/othismo:master
# http://127.0.0.1:8080
```

Prefer a `sha-…` tag for anything you treat as production.

## Fly.io (current)

[`fly.toml`](../fly.toml) still deploys this same Dockerfile (`task deploy` /
`fly deploy`). Keep that path until a home-lab cutover is ready.

## Home lab (intended)

1. On the lab host, pull a pinned tag (`sha-…` or a semver).
2. Run the container on an internal port (for example `127.0.0.1:8080:8080`).
3. Put Cloudflare Tunnel (or similar) in front of that port; keep Tailscale for
   private SSH/admin — do not publish the container port on the WAN.
4. Point `othismo.com` DNS at the tunnel once health checks look good; keep
   Fly as rollback until you are satisfied.

Those host/DNS/tunnel steps are done on the machine and Cloudflare/Tailscale
accounts; they are not performed by this repository’s CI.
