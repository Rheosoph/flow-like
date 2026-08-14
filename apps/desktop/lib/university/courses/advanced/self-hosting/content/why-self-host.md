The Customer Support Copilot pilot won over three teams in a month. Then it reached the security review, and Priya from security asked exactly one question: "When an agent presses Run, where does the customer's record actually go?" Right now the honest answer is "to a cloud we don't control." Your job this quarter is to change that answer to "it never leaves our VPC" — without breaking a single flow.

> **Predict first:** to make that true, what do you have to replace — the desktop app, the flows themselves, or the backend behind them?

## 1 · What actually moves

Only the backend. Your flows, boards, and apps don't change at all. Self-hosting swaps the services behind them: the API that authenticates users and stores app state, the SQL database, the object storage that holds metadata, content, and execution logs, and the executors that actually run flows. Model calls go to whatever providers you configure. Even the store your users browse and the app-visibility ladder they climb are served by the API you're about to own.

The client side of the migration is almost embarrassingly small. Try it right now if you have any backend URL handy:

```bash
export FLOW_LIKE_API_URL=https://your-api.example.com
./flow-like
```

That's the entire desktop migration — one environment variable, used exactly as provided. One rule comes with it: never set it to an empty string. An explicitly empty value overrides the build-time and hosted defaults and is a configuration error, not a "use the default" fallback.

## 2 · Two ways to host it

| Option | Best for | Isolation | Complexity |
|--------|----------|-----------|------------|
| Docker Compose | Single machine, development, small teams | Container | Low |
| Kubernetes (Helm chart) | Multi-node orchestration, after storage, database, and security hardening | Warm executor pods by default | Medium |

Compose runs the complete stack — web app, API, execution runtime, database, Redis, collaboration, compiler — on one Docker host. Kubernetes spreads the same components across a cluster with autoscaling and network policy options. Either way, object storage is the one piece you always bring yourself.

@CourseBanner

That's the course roadmap in one picture: a single glowing flow package at the top, wired to three destinations below — a single host stacked with containers, a three-node cluster linked in a ring, and a row of sealed capsules each holding its own flow. One workflow, three ways to run it.

## 3 · Topology is not isolation

Here's the assumption that sinks security reviews: "We'll pick Kubernetes, so every run gets its own isolated pod." Deployment topology and execution isolation are separate choices. Both Compose and Kubernetes ship with warm HTTP executors — long-lived workers that handle many runs over their lifetime — and both can instead point at configured serverless executors. The API does contain a Kubernetes Job dispatcher, but the checked-in executor's one-job runner is not implemented yet, so a fresh pod per run is not something the stock images give you today. Lesson 4 walks the full dispatch map.

**Watch out:** when you promise Priya isolation guarantees, promise what the executor actually does — not what the orchestrator could theoretically do.

**Recap**

- Self-hosting replaces the API, database, storage, and executors; flows are untouched and the desktop needs one env var.
- Compose is the low-complexity single-host stack; Kubernetes is the cluster option after hardening.
- Topology and execution isolation are independent decisions — both platforms default to warm shared workers.
