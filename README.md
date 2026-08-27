# Kairo

A Lavalink v4 compatible audio node, written in Rust. Existing Lavalink clients can point at it
without changes: it speaks the same REST API, the same WebSocket events, and the same track
encoding.

## Running it

Copy the example configuration, set a password, and start the node:

```bash
cp application.yml.example application.yml
cargo run --release
```

It listens on `0.0.0.0:2333` by default. The configuration file is taken from the first command line
argument, then `$KAIRO_CONFIG`, then `./application.yml`.

With Docker instead:

```bash
docker compose up -d
```

## Configuration

`application.yml.example` documents every key inline. The blocks worth knowing about:

- `server` and `lavalink.server`: listener, password, frame buffer, filters.
- `sources`: which platforms are enabled, their limits and credentials. Sources that only resolve
  metadata play through `sources.mirror`.
- `lyrics`: the providers to query. Disabled unmounts the lyrics endpoints.
- `logging`: level, per-module overrides, format, and optional rolling file output.
- `metrics.prometheus` and `sentry`: both off unless configured.

Every block is optional and falls back to its own defaults.

## The API

- `GET /version`, `GET /v4/info`, `GET /v4/stats`
- `GET /v4/loadtracks`, `GET /v4/loadsearch`, `GET /v4/decodetrack`, `POST /v4/decodetracks`
- `PATCH /v4/sessions/{id}`, and the player routes under `/v4/sessions/{id}/players`
- `GET /v4/routeplanner/status`, `POST /v4/routeplanner/free/{address,all}`
- `GET /v4/lyrics` and the per-player lyrics routes, when lyrics are enabled
- `GET /v4/websocket` for the event stream

Everything except the Prometheus endpoint requires the configured password in the `Authorization`
header.

## Building from source

Rust 1.96 or newer, plus `cmake`, `clang`, `pkg-config` and `perl`: libopus is compiled and linked
statically.

```bash
cargo build --release
cargo test
cargo clippy --all-targets
```

Audio decoding, source resolution, lyrics and the voice transport live in separate crates
([`player`](https://github.com/bongo-devs/player), [`sources`](https://github.com/bongo-devs/sources),
[`lyrics`](https://github.com/bongo-devs/lyrics), [`voice`](https://github.com/bongo-devs/voice)),
which Cargo fetches over git. This repository is the node around them: the REST API, the WebSocket,
the sessions and the players.

## License

MIT. See [LICENSE](LICENSE), which also carries the notice for the Lavalink protocol this node
implements.
