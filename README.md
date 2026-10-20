# Kairo

[![CI](https://img.shields.io/github/actions/workflow/status/bongo-devs/Kairo/rust.yml?branch=main&label=CI&style=for-the-badge&logo=githubactions)](https://github.com/bongo-devs/Kairo/actions/workflows/rust.yml)
[![Latest tag](https://img.shields.io/github/v/tag/bongo-devs/Kairo?sort=semver&style=for-the-badge&logo=git)](https://github.com/bongo-devs/Kairo/releases)
[![Container](https://img.shields.io/badge/GHCR-ghcr.io%2Fbongo--devs%2Fkairo-2496ED?style=for-the-badge&logo=docker&logoColor=white)](https://github.com/bongo-devs/Kairo/pkgs/container/kairo)
[![License](https://img.shields.io/github/license/bongo-devs/Kairo?style=for-the-badge)](LICENSE)

Kairo is a Rust-based audio node for Discord bots. It implements the Lavalink v4 REST and WebSocket protocol, so existing Lavalink-compatible clients can connect using the node’s address and password.

- Audio sources, filters, crossfade, lyrics, and optional Prometheus metrics
- Docker image: `ghcr.io/bongo-devs/kairo`
- [Latest release binaries for Linux, macOS, and Windows](https://github.com/bongo-devs/Kairo/releases/latest)

## Run with Docker Compose

```sh
git clone https://github.com/bongo-devs/Kairo.git
cd Kairo
cp config.example.toml config.toml
```

Edit `config.toml` before starting the node. In particular, replace the sample password and enable the sources you want. Then run:

```sh
docker compose up -d --build
```

Compose exposes port `2333`, mounts `config.toml` read-only, and sets the soft and hard `nofile` limits to `65536`. Keep this limit for a high-concurrency deployment. View container output with `docker compose logs -f`; optional file logging can use the mounted `./logs` directory.

## Run the published image

Download the example configuration, copy it, and update the password and source settings:

```sh
curl -fsSLO https://raw.githubusercontent.com/bongo-devs/Kairo/main/config.example.toml
cp config.example.toml config.toml
```

Then start the image, mounting your configuration file:

```sh
docker run -d --name kairo --restart unless-stopped \
  --ulimit nofile=65536:65536 \
  -p 2333:2333 \
  -v /absolute/path/config.toml:/app/config.toml:ro \
  ghcr.io/bongo-devs/kairo:latest
```

## Connect a client

Point your Lavalink-compatible client to `http://<host>:2333` and use the password from `lavalink.server.password`. For a quick check, request the node info endpoint:

```sh
curl -H 'Authorization: your-password' http://localhost:2333/v4/info
```

## Configuration

[`config.example.toml`](config.example.toml) documents every setting. Kairo reads the config path from the first command-line argument, then `KAIRO_CONFIG`, then `config.toml`. The `PORT` environment variable overrides `server.port`.

Most platform sources are disabled by default; the generic HTTP source is enabled. Spotify and other metadata-only sources need a playable provider configured under `sources.mirror.providers`. See the [source catalog and provider notes](https://github.com/bongo-devs/sources#sources) for supported sources, limits, and credentials.

The optional [yt-cipher](https://github.com/bongo-devs/yt-cipher) service supports YouTube client paths that need signature resolution or PO tokens. [keys-api](https://github.com/bongo-devs/keys-api) can provide shared credentials for supported sources. Both are configured in `config.toml` and are only needed for those integrations.

## Build and test

With a stable Rust toolchain and native build tools installed:

```sh
cargo build --release --locked --bin kairo
cargo test --locked
```

The Prometheus endpoint is disabled by default; when enabled, it is served at `/metrics` unless configured otherwise.

## License

Kairo is MIT licensed. See [LICENSE](LICENSE), which includes the third-party notice for the Lavalink protocol implementation.

## Star History

<a href="https://www.star-history.com/?type=date&legend=top-left&repos=bongo-devs%2FKairo">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=bongo-devs/Kairo&type=date&theme=dark&legend=top-left" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=bongo-devs/Kairo&type=date&legend=top-left" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=bongo-devs/Kairo&type=date&theme=dark&legend=top-left" />
  </picture>
</a>
