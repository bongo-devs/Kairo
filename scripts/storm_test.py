#!/usr/bin/env python3
"""Storm-test a Kairo node: 1000+ voice PATCHes plus loadtracks probes.

Exercises the voice-handshake gate and checks the node stays responsive for
search while a reconnect storm drains.

Usage:
    python3 scripts/storm_test.py --base http://localhost:2333 \
        --password youshallnotpass --session-id <sid> --players 1000

Notes (read before trusting numbers):
- Voice payloads use endpoint 127.0.0.1:1, which refuses fast. That exercises
  gate queueing and worker load, NOT a real 30 s Discord handshake. For a
  saturating variant point --endpoint at a blackhole IP, but expect the run to
  take (players / 32) * handshake-timeout seconds.
- --session-id must be a live session (open /v4/websocket with your bot first).
  Voice PATCHes to a dead session return 404 and prove nothing about the gate.
- loadtracks probes hit real sources and need network + keys-api. Failures there
  are reported by loadType so you can compare search health before/during/after.
- Stdlib only. Results JSON goes to --out for before/after diffing.

Exit code is 0 unless the harness itself errors. Pass/fail is for you to judge
from the printed summary (gate timeouts, probe p95, loadType=error rate).
"""

import argparse
import concurrent.futures
import json
import statistics
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE_GUILD = 100_000_000_000_000_000


def req(method, url, password, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("Authorization", password)
    if data:
        r.add_header("Content-Type", "application/json")
    start = time.monotonic()
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            payload = resp.read().decode()
            return resp.status, (time.monotonic() - start) * 1000, payload
    except urllib.error.HTTPError as e:
        try:
            payload = e.read().decode()
        except Exception:
            payload = ""
        return e.code, (time.monotonic() - start) * 1000, payload


def voice_patch(base, password, session, guild, endpoint, timeout):
    url = f"{base}/v4/sessions/{session}/players/{guild}"
    body = {
        "voice": {
            "token": "storm-test",
            "endpoint": endpoint,
            "sessionId": "storm-test",
            "channelId": str(guild),
        }
    }
    return req("PATCH", url, password, body, timeout)


def loadtrack(base, password, identifier, timeout):
    q = urllib.parse.quote(identifier, safe="")
    url = f"{base}/v4/loadtracks?identifier={q}"
    return req("GET", url, password, None, timeout)


def stats(base, password):
    code, _, payload = req("GET", f"{base}/v4/stats", password, None, 10)
    if code != 200:
        return {}
    try:
        return json.loads(payload)
    except Exception:
        return {}


def summarize(name, samples):
    """samples: list of (code, latency_ms)."""
    lat = sorted(s for _, s in samples)
    codes = {}
    for c, _ in samples:
        codes[c] = codes.get(c, 0) + 1
    print(f"--- {name} (n={len(samples)}) ---")
    print(f"  status: {codes}")
    if lat:
        print(
            f"  latency ms: p50={statistics.median(lat):.0f} "
            f"p95={lat[int(len(lat) * 0.95)]:.0f} max={lat[-1]:.0f}"
        )


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--base", default="http://localhost:2333")
    ap.add_argument("--password", default="youshallnotpass")
    ap.add_argument("--session-id", default="")
    ap.add_argument("--players", type=int, default=1000)
    ap.add_argument("--concurrency", type=int, default=100)
    ap.add_argument("--endpoint", default="127.0.0.1:1")
    ap.add_argument("--probe-every", type=float, default=2.0,
                    help="seconds between loadtracks probes during the storm")
    ap.add_argument("--probe-query", default="ytsearch:storm probe",
                    help="identifier for loadtracks probes")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--out", default="storm_results.json")
    ap.add_argument("--loadtracks-only", action="store_true",
                    help="skip voice storm, only run loadtracks probes")
    args = ap.parse_args()

    if not args.loadtracks_only and not args.session_id:
        ap.error("--session-id is required unless --loadtracks-only")

    result = {"args": vars(args), "phases": {}}
    print(f"node stats before: {stats(args.base, args.password)}")

    # Baseline probes (no storm).
    with concurrent.futures.ThreadPoolExecutor(max_workers=10) as ex:
        futs = [ex.submit(loadtrack, args.base, args.password,
                          f"{args.probe_query} {i}", args.timeout) for i in range(10)]
        baseline = [f.result() for f in futs]
    summarize("loadtracks baseline", [(c, l) for c, l, _ in baseline])
    result["phases"]["baseline"] = [(c, round(l)) for c, l, _ in baseline]

    if not args.loadtracks_only:
        guilds = [BASE_GUILD + i for i in range(args.players)]
        probes = []
        stop = False

        def probe_loop():
            i = 0
            while not stop:
                c, l, p = loadtrack(args.base, args.password,
                                   f"{args.probe_query} storm-{i}", args.timeout)
                try:
                    load_type = json.loads(p).get("loadType", "?") if c == 200 else f"http-{c}"
                except Exception:
                    load_type = "unparseable"
                probes.append((c, l, load_type))
                i += 1
                time.sleep(args.probe_every)

        import threading
        pt = threading.Thread(target=probe_loop, daemon=True)
        pt.start()
        t0 = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as ex:
            futs = [ex.submit(voice_patch, args.base, args.password, args.session_id,
                              g, args.endpoint, args.timeout) for g in guilds]
            voice = [f.result() for f in futs]
        dt = time.monotonic() - t0
        stop = True
        pt.join(timeout=args.timeout + 5)

        summarize("voice storm", [(c, l) for c, l, _ in voice])
        print(f"  drained {len(voice)} PATCHes in {dt:.1f}s "
              f"({len(voice) / max(dt, 1e-9):.1f}/s)")
        summarize("loadtracks during storm", [(c, l) for c, l, _ in probes])
        errs = sum(1 for _, _, t in probes if t == "error")
        print(f"  probe loadType=error: {errs}/{len(probes)}")
        result["phases"]["voice"] = {
            "seconds": round(dt, 1),
            "status": [(c, round(l)) for c, l, _ in voice],
            "probes": [(c, round(l), t) for c, l, t in probes],
        }
        print(f"node stats after: {stats(args.base, args.password)}")

    with open(args.out, "w") as f:
        json.dump(result, f, indent=2)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    sys.exit(main())
