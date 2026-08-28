# kairo

[![Build](https://img.shields.io/github/actions/workflow/status/bongo-devs/Kairo/rust.yml?branch=main&label=build)](https://github.com/bongo-devs/Kairo/actions/workflows/rust.yml)
[![Container](https://img.shields.io/badge/ghcr.io-kairo-2496ED?logo=docker&logoColor=white)](https://github.com/bongo-devs/Kairo/pkgs/container/kairo)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Standalone Discord audio sending node, written in Rust. Drop-in Lavalink v4 compatible: the same
REST API, the same WebSocket events, the same encoded track format.

## Why this exists

Kairo started as a hobby project. We wanted more control over the audio node we use, so we decided
to build our own.

We have a lot of respect for Lavalink and the people behind it. The protocol and ecosystem it built
gave us a solid foundation to work from. Without Lavalink, Kairo probably wouldn't exist.

Kairo isn't here to replace Lavalink. It's simply our own take on the problem, built because we
wanted to learn, experiment, and make something of our own.

## Running

Docker is the supported way to run it.

```sh
curl -O https://raw.githubusercontent.com/bongo-devs/Kairo/main/application.yml.example
mv application.yml.example application.yml

docker run -d --name kairo -p 2333:2333 \
  -v "$PWD/application.yml:/app/application.yml:ro" \
  -v "$PWD/logs:/app/logs" \
  ghcr.io/bongo-devs/kairo:latest
```

Or, from a clone of this repository:

```sh
cp application.yml.example application.yml
docker compose up -d
```

The node serves port 2333 and reads `/app/application.yml`. Set `KAIRO_CONFIG` to read it from
somewhere else.

## Configuration

Every key is documented inline in [`application.yml.example`](application.yml.example). The blocks:

- `server`, `lavalink.server`: the listener, the client password, the frame buffer, the filters.
- `sources`: which platforms are enabled, and their limits and credentials. A source that only
  resolves metadata plays through `sources.mirror`.
- `lyrics`: the providers to query. Disabled leaves the lyrics endpoints unmounted.
- `logging`: level, per-module overrides, format, and optional rolling file output.
- `metrics.prometheus`: off until configured.

Every block is optional and falls back to its own defaults. All endpoints except the Prometheus one
require the configured password in the `Authorization` header.

### Companion services

- yt-cipher, <https://github.com/bongo-devs/yt-cipher>, solves the signature cipher and mints the
  poTokens SABR playback needs. Configured under `sources.youtube.remoteCipher`. Use this fork
  rather than the upstream project, which exposes different endpoint names.
- keys-api, <https://github.com/bongo-devs/keys-api>, is a shared credential store for the Deezer
  `arl`, YouTube OAuth tokens and cookies. Configured under `sources.keysApi`, and sources take
  their credentials from there instead of the config file.

## Credits

The REST API, the WebSocket events and the encoded track format are those of
[Lavalink](https://github.com/lavalink-devs/Lavalink) by Freya Arbjerg and its contributors, which
defines the v4 protocol this node implements and which every compatible client was written against.
Lavalink is MIT licensed; its notice is reproduced in [LICENSE](LICENSE).

## License

MIT. See [LICENSE](LICENSE), which also carries the third-party notice for Lavalink.
