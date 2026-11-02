# grok-box

A sandboxed Linux computer for agents: a guest Docker image, two HTTP daemons (`box-exec` and `box-host`), a `grok-box` CLI, and connect-only SDKs (Rust, TypeScript, Python).

You start the guest. Then you talk to it over HTTP with a URL pair and a bearer token. This repo does **not** publish packages to crates.io, npm, or PyPI.

License: **MIT**. MSRV: Rust **1.85**.

## What you get

| Piece | Role |
| --- | --- |
| Guest image `grok-box` | Linux box: shell, files, 1280×800 X desktop, Chromium, Computer Use |
| `box-exec` `:1337` | Exec, files (GET/PUT/DELETE/mkdir), CUA |
| `box-host` `:1340` | Health, ready, identity, desktop/chrome/egress status |
| CLI `grok-box` | Same surface as the SDKs |
| CLI `grok-box` | Same surface as the SDKs |
| SDKs | Connect with `(execUrl, hostUrl, token)` — no `docker run` helper |
| [`ensurebox/`](ensurebox/) | **Frozen demo / non-product** — sample orchestrator / operator UI (not a supported control plane) |
| [`l1/`](l1/) | **Frozen demo / non-product** — sample human workspace UI (talks only to EnsureBox) |

Windows and macOS are clients or Docker hosts. The box OS is always this Linux image.

This tree implements **our own HTTP wire**. It is not Cursor’s `/exec-daemon` or `sand-host`.

## Quick start (guest)

```bash
cp .env.example .env            # set BOX_TOKEN and BOX_VNC_PASSWORD (required)
bash scripts/write-secrets.sh   # turns them into ./secrets/* for Compose to mount
docker compose up --build
```

That builds the image and starts exec, host, Xvfb, Chromium, and CUA tools. Compose publishes **1337 / 1340 / 6080 on 127.0.0.1**. Reach them from another machine with an SSH tunnel, Tailscale, or similar — do not publish those ports on a public NIC.

Health (no token). Ready requires Bearer:

```bash
curl -fsS http://127.0.0.1:1337/v1/health
curl -fsS http://127.0.0.1:1340/v1/health
curl -fsS -H "Authorization: Bearer $BOX_TOKEN" http://127.0.0.1:1340/v1/ready
```

Exec (token required):

```bash
curl -fsS http://127.0.0.1:1337/v1/exec \
  -H "Authorization: Bearer $BOX_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"command":["echo","ok"]}'
```

Screenshot as JSON (base64 PNG) or raw `image/png`:

```bash
curl -fsS http://127.0.0.1:1337/v1/cua/screenshot \
  -H "Authorization: Bearer $BOX_TOKEN" \
  -X POST

curl -fsS http://127.0.0.1:1337/v1/cua/screenshot?format=png \
  -H "Authorization: Bearer $BOX_TOKEN" \
  -H "Accept: image/png" \
  -X POST \
  -o /tmp/box.png
```

CUA coordinate space is **1280×800**, origin top-left. Known click/type/key sequences should go in **one** `POST /v1/cua/recipe` (see [RECIPES.md](docs/RECIPES.md)) instead of one HTTP call per action.

Mixed native+CUA programs can now run in **one** `POST /v1/recipe` request (including shell, files, graphical recipes, assertions, and durable pending-effect journals). See [Mixed guest programs](docs/PROGRAM_RECIPES.md), [the example](examples/guest-program.json), and [the reproducible HTTP comparison](examples/benchmark_guest_program.py). This endpoint keeps the existing guest Bearer and workspace semantics; it is not a semantic world-model compiler.

A receipt's `ok` means **no step returned an error**, not that the recipe achieved anything: a taped plan whose fixed coordinates have drifted onto another window plays every step cleanly. Ask for `observe: "input"` (or `"page"`) and each step also reports the window it aimed at, where the keys were about to go, and the page URL either side of it, so a caller can tell a run that worked from a run that did not.

Desktop viewer: [http://127.0.0.1:6080/vnc.html](http://127.0.0.1:6080/vnc.html) (loopback publish). Password is `BOX_VNC_PASSWORD` (x11vnc uses the first **8** characters). It is independent of `BOX_TOKEN`. 6080 is **not** Bearer-authenticated — firewall + loopback bind are the control. x11vnc listens on `127.0.0.1:5900` **inside** the container.

```bash
./scripts/smoke.sh
```

### CLI and SDKs

After the guest is up, any orchestrator (yours, or the EnsureBox demo) already has an exec URL, host URL, and token. Point the client at those. The SDKs **do not** start Docker and **ignore** `/v1/info` advertised URLs (those are container-local listen addresses).

```bash
cargo run -p grok-box -- \
  --exec-url http://127.0.0.1:1337 \
  --host-url http://127.0.0.1:1340 \
  --token "$BOX_TOKEN" \
  exec -- echo ok
```

Workspace packages (not published):

- Rust: `crates/grok-box` (library + `grok-box` binary)
- TypeScript: `sdk/typescript`
- Python: `sdk/python`

### Native (no Docker, no X desktop)

```bash
./scripts/run-local.sh
# another terminal
./scripts/smoke-native.sh
cargo test --workspace
```

Native mode sets `BOX_DESKTOP=0`. CUA screenshot needs the container (or a local Xvfb).

## How an orchestrator should plug in

1. Start a container from this image. Inject `BOX_TOKEN`, `BOX_VNC_PASSWORD`, and `BOX_ID`. Publish **1337 / 1340 / 6080** on loopback (or behind a tunnel). Do **not** publish 5900 or 9222. Optional laptop egress: [EGRESS.md](docs/EGRESS.md) (`BOX_EGRESS_TUNNEL=1`, publish **8790** only, never 8791).
2. Wait until `GET <hostUrl>/v1/ready` returns 200 **with Bearer**.
3. Call `connect(execUrl, hostUrl, token)` in the CLI or an SDK. Do not parse `/v1/info.endpoints` as the public URLs.
4. Drive `POST /v1/exec`, files, `/v1/cua/*`, and `POST /v1/cua/recipe` yourself.

[`ensurebox/`](ensurebox/) is a **demo** of that pattern. It is not a supported production control plane. [`l1/`](l1/) is a **demo** human UI that talks only to EnsureBox (`ENSUREBOX_TOKEN` server-side, `L1_TOKEN` for the browser session). L1 never sees `BOX_TOKEN`, never SSHes, and never calls `box-exec` / `box-host`.

Demo UIs (optional; bind 127.0.0.1):

```bash
cd ensurebox && cp .env.example .env && npm install && npm run dev   # operator console, :43142
cd l1 && cp .env.example .env && npm install && npm run dev          # human workspace, :43141
```

`.env.example` uses well-known demo tokens with `*_ALLOW_INSECURE_DEV=1` for loopback only. Replace them for anything else.

## Ports

| Port | Published? | Process | Notes |
| --- | --- | --- | --- |
| **1337** | host `127.0.0.1` | `box-exec` | exec, files, CUA. Process bind inside the image is `0.0.0.0`. |
| **1340** | host `127.0.0.1` | `box-host` | health, ready, info, desktop, chrome, egress. |
| **6080** | host `127.0.0.1` | websockify / noVNC | Viewer. **Not** Bearer-authenticated. |
| **8790** | host `127.0.0.1` | `box-egress-tunnel` WS | Laptop client. Off unless `BOX_EGRESS_TUNNEL=1`. See [EGRESS.md](docs/EGRESS.md). |
| 8791 | **no** | CONNECT proxy | Chromium only, `127.0.0.1` inside the image. Never publish. |
| 8792 | **no** | tunnel admin | Status for box-host. Loopback. Never publish. |
| 5900 | **no** | x11vnc | `BOX_VNC_BIND=127.0.0.1:5900` inside the image |
| 9222 | **no** | Chromium CDP | `127.0.0.1` only (`BOX_CDP_PORT`). Do not publish. |

Compose also sets `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]`, `pids_limit: 1024`, and `mem_limit: 4g`. That is **not** a kernel sandbox. The guest is an unprivileged uid-1000 container; Chromium still uses `--no-sandbox`.

## Environment

| Variable | Default (image) | Meaning |
| --- | --- | --- |
| `BOX_TOKEN` | **required** | Bearer token for exec, CUA, and host `/v1/info` / `/v1/ready`. No silent default. |
| `BOX_TOKEN_FILE` | unset | Path to a file holding `BOX_TOKEN`. **Preferred**: wins over `BOX_TOKEN`, and keeps the value out of `/proc/<pid>/environ`. |
| `BOX_HOST_TOKEN` | same as `BOX_TOKEN` | Optional split token for host info/desktop/chrome |
| `BOX_HOST_TOKEN_FILE` | unset | Path to a file holding `BOX_HOST_TOKEN` |
| `BOX_ALLOW_INSECURE_DEV` | unset | `1` allows short/well-known tokens **only** when both daemon binds are loopback |
| `BOX_ID` | hostname / `grok-box` | Reported by `/v1/info` |
| `WORKSPACE_ROOT` | `/workspace` | Jail root for cwd and file APIs |
| `BOX_EXEC_BIND` | `0.0.0.0:1337` in image; `127.0.0.1:1337` native | Exec listen address |
| `BOX_HOST_BIND` | `0.0.0.0:1340` in image; `127.0.0.1:1340` native | Host listen address |
| `BOX_EXEC_URL` | `http://127.0.0.1:1337` | URL host uses to probe exec (container-local) |
| `BOX_CORS_ORIGINS` | empty (no browser origins) | Comma-separated allowlist; `*` is ignored |
| `BOX_DISPLAY` | `:1` | X display |
| `BOX_DISPLAY_GEOM` | `1280x800x24` | Xvfb geometry; **CUA coordinate space is 1280×800** |
| `BOX_DESKTOP` | `1` | Start Xvfb + openbox + x11vnc + noVNC |
| `BOX_DESKTOP_REQUIRED` | `1` when desktop on | `/v1/ready` waits for the display |
| `BOX_VNC_BIND` | `127.0.0.1:5900` | x11vnc (localhost only) |
| `BOX_NOVNC_PORT` | `6080` | noVNC / websockify |
| `BOX_VNC_PASSWORD` | **required when desktop on** | Viewer password; independent of `BOX_TOKEN`; x11vnc uses 8 chars |
| `BOX_VNC_PASSWORD_FILE` | unset | Path to a file holding `BOX_VNC_PASSWORD`. Preferred, same reason. |
| `BOX_CHROME` | `1` | Launch Chromium on `:1` |
| `BOX_CHROME_PROFILE` | `/home/box/chrome-profile` | Persistent profile (compose volume) |
| `BOX_CDP_PORT` | `9222` | CDP on `127.0.0.1` only |
| `BOX_CUA` | `1` | Enable `/v1/cua/*` |
| `BOX_EGRESS_TUNNEL` | `0` | `1` starts `box-egress-tunnel` before Chromium and sets `--proxy-server`. See [EGRESS.md](docs/EGRESS.md). |
| `BOX_EGRESS_WS_BIND` | `0.0.0.0:8790` in image; `127.0.0.1:8790` native | WebSocket for the laptop client |
| `BOX_EGRESS_PROXY_BIND` | `127.0.0.1:8791` | HTTP CONNECT for Chromium. Never publish. |
| `BOX_EGRESS_ADMIN_BIND` | `127.0.0.1:8792` | Loopback status for box-host |
| `BOX_EGRESS_TUNNEL_BEARER` | required when tunnel on | WS Bearer. Prefer `_FILE`. Independent of `BOX_TOKEN`. |
| `BOX_EGRESS_TUNNEL_BEARER_FILE` | unset | Preferred file form (same story as `BOX_TOKEN_FILE`) |
| `BOX_EGRESS_RELAY_HOSTS` | empty = all | Optional CONNECT host allowlist (`*.example.com` ok) |
| `BOX_MAX_CONCURRENT_EXECS` | `8` | Max simultaneous `POST /v1/exec` (and stream/detach). Extra calls get `429 busy`. |
| `BOX_MAX_CONCURRENT_EXECS` | `8` | Max simultaneous `POST /v1/exec` (and stream/detach). Extra calls get `429 busy`. |
| `BOX_MAX_DIR_ENTRIES` | `4096` | Cap on directory listings (`truncated: true` if hit) |
| `BOX_EXEC_KILL_GRACE_MS` | `2000` | After exec timeout, wait this long after SIGTERM before SIGKILL |

## Auth

Send `Authorization: Bearer <token>`. `GET /v1/health` is public and returns only `{"status":"ok"}`. `GET /v1/ready` requires Bearer (Compose healthcheck sends it). A token must be set — as `BOX_TOKEN` or as `BOX_TOKEN_FILE`; `dev-box-token` and other short/well-known values are rejected unless `BOX_ALLOW_INSECURE_DEV=1` **and** both daemon binds are loopback.

### Delivering the secrets

Pass each secret as a **file**, not as a value: `BOX_TOKEN_FILE`, `BOX_HOST_TOKEN_FILE`, `BOX_VNC_PASSWORD_FILE`, `BOX_EGRESS_TUNNEL_BEARER_FILE`. The file form wins when both are set. Compose does this for you — `scripts/write-secrets.sh` writes `./secrets/*` and Compose mounts them read-only at `/run/secrets/`.

A secret handed to the container as an **environment value** is copied into pid 1's environment block at `execve`. `/proc/1/environ` serves that block to every process in the box, for the life of the box, and `docker inspect` shows it on the host. Nothing inside the box can undo that. In particular `wipe_secret_environ()` does **not**: `unsetenv` rewrites the `environ` pointer array but leaves the original block on the stack, and that block is what `/proc` reads. What it does do is keep the value out of `getenv` for the rest of the process, which is worth having but is not the same claim.

What the file form buys, precisely:

- `grep -a BOX_TOKEN= /proc/*/environ` finds nothing — not in pid 1, not in either daemon, not in Chromium.
- `docker inspect` does not show the token in `Config.Env`.
- The daemons unlink the staged copies the entrypoint writes for them, so those exist for milliseconds.

What it does **not** buy: the file the operator mounts stays readable by uid 1000 (the Compose healthcheck needs it to authenticate). Code already executing as the box user — a compromised Chromium renderer, or any binary the agent downloaded and ran — can read `/run/secrets/box_token` and recover both secrets. Treat any code execution in the box as full compromise of `BOX_TOKEN` and `BOX_VNC_PASSWORD`. Rotate on suspicion; do not reuse a box token anywhere else.

Exec children never receive any of these variables, in either form. Do not put `BOX_TOKEN` in L1.

## Layout

```
crates/box-common    path jail, bearer compare, error envelope, config, CORS
crates/box-exec      exec + files + CUA HTTP daemon
crates/box-host      identity / ready / capabilities / desktop + chrome status
crates/box-desktop   Xvfb probe, 1280×800 geometry, viewer URL
crates/box-chrome    Chromium profile + localhost CDP probe
crates/box-cua       screenshot / click / type / key / scroll / double-click / drag / move / recipe
crates/box-egress-tunnel  guest WS mux + Chromium CONNECT proxy (laptop client is the same binary)
crates/grok-box      typed client + CLI (workspace only, not published)
sdk/typescript       TypeScript client (workspace only)
sdk/python           Python client (workspace only)
ensurebox/           demo orchestrator (not production)
l1/                  demo human UI (EnsureBox only)
```

## Volumes

| Mount | Role |
| --- | --- |
| `/workspace` | Jail root for cwd and file APIs. Persist across hibernate. |
| `/home/box/chrome-profile` | Chromium `--user-data-dir`. Persist cookies/session. Must be writable by uid **1000**; otherwise the entrypoint falls back to `/tmp/box-chrome-profile`. |

## Docs

- [Deploy](docs/DEPLOY.md) — test-deploy on **Akamai Cloud** (Linode; not Vultr), then the same pattern on AWS/GCP/Azure. First test is one VM, guest Compose only, private SSH tunnel or Tailscale; CLI/SDK from your laptop. Do not expose 1337/1340/6080 on the public internet.
- [Egress](docs/EGRESS.md) — laptop CONNECT tunnel for Chromium in prod (no host-network)
- [Architecture](docs/ARCHITECTURE.md)
- [Startup](docs/STARTUP.md)
- [Request processing](docs/PROCESSING.md)
- [Terminology](docs/TERMINOLOGY.md)
- [HTTP API](docs/API.md) · [OpenAPI](docs/openapi.yaml)
- [CUA recipes](docs/RECIPES.md) — one request, many pointer steps (vs reverse-web-mcp)

## What this is not

- Not a supported production control plane (EnsureBox is a demo)
- Not L4 / not an OpenAI-compatible inference gateway
- Not a re-host of any proprietary exec/sand-host / sand-egress-tunnel binary
- Not a native Windows or macOS box OS
- Not a vendored [trycua/cua](https://github.com/trycua/cua) tree — Linux X11 is the CUA backend
- Not published to crates.io, npm, or PyPI
