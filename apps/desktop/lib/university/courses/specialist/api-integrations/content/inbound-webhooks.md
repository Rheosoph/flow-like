Pulling context is half the loop. Orbit still knows things first: Mia upgrades her plan at 2 a.m., and your app finds out at 9:15 when somebody happens to triage her next ticket. Better if Orbit tells you the moment it happens — by making an HTTP request *to your app*. That's the entire idea of a webhook: this time, you're the API.

> **Predict first:** Your laptop is asleep at 2 a.m. Where must the webhook event execute — Local or Remote — for Orbit's notification to land anyway?

## 1 · Give the flow an entry

An inbound request needs somewhere to arrive. In the flow, that's an event entry node — a **Generic Event** node fits here, because Orbit sends a structured JSON payload and Generic maps named fields into typed output pins. The Events course owns entry contracts and the Create Event workflow; here we only use them.

@AppEvents

The Events workspace for the Customer Support Copilot currently lists two UI events — the *Triage selected request* Quick Action at `/triage` and the *Support assistant* Chat UI at `/chat`. **Create Event** is where the webhook starts: select the flow and its Generic Event node, then choose the **API** type. An API event exposes exactly one configured HTTP endpoint — precisely webhook-shaped. It's backend-only, so it needs no UI route badge like its two neighbors.

## 2 · The endpoint contract

You configure a path (it must start with `/`) and an HTTP method. Where the URL lives depends on execution location:

- **Local**: the desktop app serves an HTTP listener on port 9657 — `http://localhost:9657/{app_id}/orbit-events`. Perfect for testing from a terminal.
- **Remote**: the hosted backend exposes it publicly under `/sink/trigger/http/{app_id}/orbit-events`.

Try the local one while you build:

```bash
curl -X POST "http://localhost:9657/<your-app-id>/orbit-events" \
  -H "Content-Type: application/json" \
  -d '{"event": "customer.updated", "email": "mia@example.com", "plan": "scale"}'
```

The response streams the run's progress as server-sent events, with the final result in the last event — your caller can watch the flow execute. A request to a path no sink matches gets a `404`.

## 3 · Lock the door

The endpoint takes an optional bearer auth token. Configure one and every request must carry it in the `Authorization` header — anything else gets a `401 Unauthorized`. Skip the token only when the sender genuinely can't set headers, or in throwaway dev setups; a production webhook without one is a public trigger for anyone who finds the URL.

And even with the token: validate the payload *inside the flow*. The endpoint is a boundary. Nothing guarantees a request came from Orbit just because it's shaped like Orbit's — replayed, malformed, or hand-crafted requests reach your Generic Event node exactly like real ones. Check the fields you depend on before acting on them; unexpected shapes go to a log, not into ticket records.

## 4 · Local or remote

API events run **Local or Remote** — your choice in the event editor. Local means the desktop app must be running, which is exactly right while you develop against curl. Remote answers the prediction: the hosted endpoint accepts Orbit's 2 a.m. notification while every laptop in the company sleeps.

Moving to Remote changes one more thing, and you learned it in the secrets lesson: a remote run receives no locally stored secret values. If the webhook flow calls back into Orbit — say, to fetch the full customer record when a notification arrives — its credential must be provisioned server-side, not borrowed from your laptop's Runtime Variables screen.

## Recap

- Webhook = API event: one configured HTTP endpoint (path + method) pointing at an event entry node in your flow.
- Local serves on port 9657 for development; Remote serves a public hosted endpoint that works while desktops sleep.
- Protect it twice: a bearer token at the door, and payload validation inside the flow.
