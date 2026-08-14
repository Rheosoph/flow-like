Friday, 16:55. A teammate "just tidies up" the draft of the intake flow — renames a pin, simplifies the validation branch — and heads into the weekend. Nothing about the `/support/new` event changed: same route, same node, same configuration. Monday morning, every new ticket arrives without a priority.

What happened? The event targets **Latest**. Hold that thought while the support app grows its last surfaces — the answer closes the lesson.

## 1 · Chat surfaces

The "Support assistant" you saw in lesson one is a **Chat UI** event on a **Chat Event** node. That node receives real conversation context — history, local and global session values, tools, actions, attachments, user information — and chat response nodes stream text, steps, attachments, or widgets back into the interface.

**Discord** and **Telegram** target the same Chat Event contract, but their sinks hold long-lived service connections and are local in the current availability model. Budget for token management, message scopes, channel allow/deny rules, and mention behavior.

Whatever the surface: don't shovel every history item and attachment into a model automatically. Enforce size, type, access, and retention limits — user content and tool output are untrusted input wearing a friendly face.

## 2 · Mail and deep links

An **Email** event enters through the **Mail Event** node and is local. Give it a least-privileged mail account, define exactly which mailboxes and messages may invoke the flow, and — the classic — make sure the flow never answers its own output. An auto-reply that triggers on its own auto-reply is an infinite loop with a mail server in the middle.

A **Deeplink** targets a Generic or Simple Event node and is a desktop invocation path. Deep-link parameters are untrusted, and an unguessable route is a convenience, not authorization — validate the active user and every referenced resource, same as any other entry.

## 3 · Latest versus pinned

Now the Friday mystery. An event can target:

- **Latest** — follows the *editable draft*. Every saved edit is immediately live for that event.
- A **numbered version** — an immutable snapshot. Nobody can change what it does, only which events point at it.

The teammate's tidy-up shipped to production the moment they saved, because Latest is a moving target. That's a feature during development — edit, press the Quick Action, edit again — and a liability for anything with users or partners on the other end.

Look back at the event edit view from the routes lesson:

@RouteConfiguration

That **Flow Version** dropdown in the Flow Configuration card — reading *Latest* — is the entire blast-radius decision, one click wide. Pin production-facing API, Cron, REST, MCP, mail, bot, and business-critical UI events to a reviewed numbered version.

## 4 · Rollout, the boring safe way

With versions pinned, change becomes a deliberate sequence:

1. Fix and test the draft, then create a new numbered version.
2. Point a temporary or internal event at it; test ordinary, boundary, unauthorized, and dependency-failure cases through the real surface — chat formatting and mail behavior can't be proven by a board run alone.
3. Repoint the production event during a controlled window and watch failures, latency, and business outcomes.
4. Rollback = repoint to the previous version. You never edit an immutable version — you make a better one.

> **Watch out:** pinning freezes your flow, not the world. Providers, schemas, and credentials the flow depends on keep changing — keep monitoring them.

## Recap

- Chat UI, Discord, and Telegram share the Chat Event contract; Email enters through Mail; Deeplinks are desktop paths with untrusted parameters.
- Latest follows the draft — great for building, dangerous for production surfaces.
- Rollout = new version, canary event, repoint; rollback = repoint back.
