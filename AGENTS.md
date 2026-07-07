# Agent Guide

Guidance for AI agents working in this repository.

## Local services & ports

This project runs two long-lived dev servers:

| Service | Command | Port |
|---|---|---|
| Backend (`quasar`) | `./target/debug/quasar` (or `QUASAR_BIND=127.0.0.1:<port>`) | `3000` |
| Frontend (webpack dev server) | `npm run dev` / `npx webpack serve` (in `apps/frontend`) | `5173` |

The frontend proxies `/api` → `http://127.0.0.1:3000`.

## Policy: always clean up background servers you start

**Incident:** an agent started a frontend dev server on `:5173` (and a backend on `:3000`) to take a screenshot, then left it running. The next attempt to start a dev server failed with `Error: listen EADDRINUSE: address already in use :::5173`.

Follow these rules whenever you launch a server, watcher, or any long-running background process:

1. **Prefer an alternate port** for throwaway/verification runs so you never collide with the user's own servers, e.g. `QUASAR_BIND=127.0.0.1:3090`. The frontend proxy is hardcoded to `:3000`, so only screenshot through the frontend when you genuinely need it.
2. **Track the PID** of anything you start in the background (`echo $! > /tmp/<name>.pid`).
3. **Kill it when you're done** in the same task — do not leave it running across turns. Note that `webpack serve` / `webpack-dev-server` forks a **detached child**, so `kill <parent>` may not reap the listener; kill by port to be sure.
4. **Before starting a server, check the port is free**, and **after killing, verify** it was released.
5. If you must restart a server the user was already running (e.g. to pick up a rebuilt binary), say so explicitly in your response.

### Free a stuck port

```bash
# Replace 5173 with 3000 as needed
kill $(ss -ltnp 2>/dev/null | grep ':5173' | grep -oP 'pid=\K[0-9]+') 2>/dev/null
ss -ltn 2>/dev/null | grep -q ':5173' && echo "STILL IN USE" || echo "free"
```
