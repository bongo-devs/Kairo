# kairo

A Lavalink v4 compatible audio node. Existing Lavalink clients talk to it unchanged: the same REST
API, the same WebSocket events, the same encoded track format.

Track loading, decoding, lyrics and the voice connection come from the
[`player`](https://github.com/bongo-devs/player), [`sources`](https://github.com/bongo-devs/sources),
[`lyrics`](https://github.com/bongo-devs/lyrics) and [`voice`](https://github.com/bongo-devs/voice)
crates. This repository is the node around them: the HTTP API, the WebSocket, and the sessions and
players behind both.

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
- `metrics.prometheus`, `sentry`: both off until configured.

Every block is optional and falls back to its own defaults. All endpoints except the Prometheus one
require the configured password in the `Authorization` header.

## Credits

The REST API, the WebSocket events and the encoded track format are those of
[Lavalink](https://github.com/lavalink-devs/Lavalink) by Freya Arbjerg and its contributors, which
defines the v4 protocol this node implements and which every compatible client was written against.
Lavalink is MIT licensed; its notice is reproduced in [LICENSE](LICENSE).

## License

MIT. See [LICENSE](LICENSE), which also carries the third-party notice for Lavalink.
