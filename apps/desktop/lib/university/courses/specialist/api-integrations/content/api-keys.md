The webhook lands, the pull works — and now Orbit's engineers want more. They'd like to query ticket summaries from their backend, trigger a re-sync flow on demand, maybe page through data on a schedule. Their ask is one sentence: "Can we get an API key?" Your app has an entire page built for this conversation.

> **Predict first:** Flow-Like offers two caller credentials — a Personal Access Token created in *your user settings*, and a Technical User API Key created in *the app's Team settings*. Which one should Orbit's production backend use?

## 1 · One endpoint, or a surface

The API event from last lesson is one endpoint — ideal for a webhook, cramped for a service. When a consumer needs several authenticated endpoints that behave as one coherent thing, that's the **REST** event type: a multi-endpoint REST surface with authentication, which runs **Remote** only. Its sibling **MCP** exposes a Model Context Protocol server for tool-capable AI clients — same remote-only rule.

Both come with a visibility choice: **Public** or **Internal**. Internal exposes no public endpoint at all — it's callable only by connected apps through the app-connection proxy. Use it when the consumer is another Flow-Like app rather than the open internet.

Stacking API events never adds up to a REST surface, in either direction: one webhook is an API event, a service is a REST event.

## 2 · The Endpoints page

In the app sidebar, open **Endpoints**. This page is your integration handout for Orbit's engineers:

- **SDK installation** — TypeScript and Python snippets that trigger events, upload files, and query tables against this app.
- **Authentication** — both credential formats, documented side by side.
- **App Endpoints** — every endpoint scoped to this app, with the app ID pre-filled in the paths.
- **Full API documentation** — a button that opens Swagger UI for the complete interactive reference.

The authentication card resolves the prediction. A **Personal Access Token** (`pat_{id}.{secret}`) is created in your user settings and is right for personal scripts and development — it authenticates *as you*, with your access. A **Technical User API Key** (`flk_{app_id}.{key_id}.{secret}`) is created in the app's Team settings and is built for server-to-server integrations: scoped to this app, tied to a technical user instead of a person. Give Orbit's backend the technical-user key. If it borrowed your PAT, the integration would act with your permissions — and die the day your account changes.

## 3 · Pin the contract

One more thing before Orbit's parser depends on your response shapes.

@RouteConfiguration

The screenshot shows an event open in editing mode — name, description, and route editable on the left, and on the right the Flow Configuration panel with its **Flow Version** dropdown reading **Latest**, above the editing bar's Save Changes button. Latest follows the editable draft: every save to the flow is instantly live for events that target it. That's convenient while you iterate and quietly dangerous once an external consumer relies on the event — a teammate's Tuesday-night draft edit becomes Orbit's Wednesday-morning parser failure, with no event change anywhere in sight.

Before handing over credentials, point every externally consumed event at a numbered, immutable flow version. The Events course covers the versioning workflow in depth; here it's simply the difference between "our draft" and "their contract".

> **Watch out:** A key is a password with better branding. Rotate it on a schedule, revoke it when the integration or its owner leaves, and never let it into a board, a screenshot, or a committed config file.

## Recap

- API event = one endpoint; REST event = an authenticated multi-endpoint surface (Remote only, Public or Internal); MCP serves tool-capable AI clients.
- The Endpoints page hands consumers everything: SDK snippets, app-scoped endpoint list, Swagger UI, and both credential formats.
- PATs act as a person; technical-user API keys belong to the app — and externally consumed events get pinned versions, not Latest.
