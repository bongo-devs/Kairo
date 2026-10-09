# Changelog

## Unreleased

### Behavior change: voice PATCH no longer blocks on the Discord handshake

`PATCH /v4/sessions/{sessionId}/players/{guildId}` with a `voice` object now
returns `200` with the current player snapshot immediately and connects in the
background. Previously it blocked until the gateway + UDP handshake finished
(up to 30 s) or failed.

What this means for clients:

- A `200` means "accepted, connecting", not "connected". Watch `connected` in
  `playerUpdate` / `GET player`: it flips to `true` on success (the commit sends
  an update, and the gateway-ready event sends another).
- A `connected: false` that never flips is a failed connect. The failure is
  server-logged with guild id, elapsed time and cause. Re-PATCH to retry; there
  is no failure event in the v4 protocol.
- Voice validation errors are still synchronous (`400` for an incomplete voice
  object).
- A PATCH carrying voice + track may start playback before the connection is
  ready. Frames wait in the track buffer until the send loop drains them.

Why: a large bot reconnecting 1000+ guilds at once opened that many simultaneous
TLS + UDP handshakes, starving search/playback HTTP while voice alone looked
connected. Background connects drain through a 32-slot gate instead of failing
the queue tail.

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
