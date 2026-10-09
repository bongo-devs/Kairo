# Changelog

## Unreleased

### Session resume matches upstream: no timeout clamp, unbounded event replay

- `PATCH /v4/sessions/{sessionId}` stores `timeout` as-is (seconds, like
  upstream) instead of clamping to 3600 s. A client sending `360000` parks its
  session for that long on both servers now; negative values expire at once.
- A paused session queues every outbound message with no cap and replays the
  whole backlog on resume, like upstream — previously the oldest events past
  4096 were dropped and player updates were skipped, losing `trackStart` /
  `trackEnd` on long gaps exactly when the client needed them to catch up.
  Parked sessions pin their players and queued events until the timeout, same
  trade-off as upstream; operators control it via the timeout they send.

### Behavior change: voice PATCH handshake mode (`backgroundConnect`)

`PATCH /v4/sessions/{sessionId}/players/{guildId}` with a `voice` object
follows `lavalink.server.voice.backgroundConnect` (default `false`, classic
blocking behavior):

- `false`: the PATCH waits for one gated handshake and returns the connected
  snapshot; handshake errors return `500`/`503` from the PATCH itself, as before.
- `true`: the PATCH returns `200` with the current snapshot immediately and
  connects in the background.

What this means for clients when background connects are enabled:

- A `200` means "accepted, connecting", not "connected". Watch `connected` in
  `playerUpdate` / `GET player`: it flips to `true` on success (the commit sends
  an update, and the gateway-ready event sends another).
- A failed connect retries up to 3 times with 500 ms doubling backoff, then the
  session emits a `WebSocketClosedEvent` (`code` 1006, `by_remote: true`,
  reason names the attempt count). There is no other failure event in v4, so a
  `connected: false` that never flips and no close event means still connecting.
- Voice validation errors are still synchronous (`400` for an incomplete voice
  object).
- A PATCH carrying voice + track may start playback before the connection is
  ready. Frames wait in the track buffer until the send loop drains them.

Why the background mode exists: a large bot reconnecting 1000+ guilds at once
opened that many simultaneous TLS + UDP handshakes, starving search/playback
HTTP while voice alone looked connected. Background connects drain through a
32-slot gate instead of failing the queue tail.

### New config: `lavalink.server.voice.*`

```yaml
lavalink:
  server:
    voice:
      maxConcurrentHandshakes: 32 # live handshakes; rest queue without holding a worker
      queueWarnMs: 15000          # warn when a queued connect waits past this (wait is unbounded)
      handshakeTimeoutMs: 10000   # outer bound per handshake; voice crate aborts inside at 30s
```

### New Prometheus metrics

- `kairo_voice_handshakes_total`, `kairo_voice_handshake_queue_timeouts_total`
- `kairo_voice_handshake_success_total`, `kairo_voice_handshake_timeout_total`,
  `kairo_voice_handshake_error_total`, `kairo_voice_handshake_stale_dropped_total`,
  `kairo_voice_handshake_retries_total`
- `kairo_voice_handshakes_in_flight`, `kairo_voice_handshake_queue_waiting`
- `kairo_voice_queue_wait` / `kairo_voice_handshake_duration` histograms (ms)
- `kairo_runtime_workers`, `kairo_runtime_alive_tasks`,
  `kairo_runtime_global_queue_depth`,
  `kairo_runtime_worker_busy_seconds_total{worker}` (use `rate()` for busy %)

### New 5 s / 30 s server logs

- `runtime metrics`: workers, alive tasks, global queue, worker busy %.
  Raise `worker_threads` only if `max_busy_pct` stays high with
  `global_queue > 0` and no blocking call is left on workers.
- `resource monitor`: fd count/limit, players, gate in-flight/waiting/timeouts.

### Transport failures carry structured causes (needs player v0.1.9)

Player v0.1.9 records the full reqwest source chain (`timeout=… connect=… io
kind=… os=…`) instead of bare `error sending request for url (…)`, and Kairo
parses those tokens (anything older classifies as `unknown`, not guessed).
This branch still builds against player v0.1.8 until a sources release built on
player v0.1.9 exists; the parser degrades gracefully meanwhile. Bump steps are
in the release notes.
