The first draft of this recipe made a memorable mistake. It answered every mail in support@ — including the out-of-office autoreplies — and then answered them all again fifteen minutes later.

> **Predict first:** the flow classified perfectly and drafted lovely replies. What one thing was missing that made it repeat itself every sweep?

By the end of this lesson you'll build the fixed version: a triage recipe that files invoices, flags bugs, drafts replies — and never touches the same mail twice.

## 1 · The chore

support@brightbeam.io collects invoices, bug reports, partner questions, and spam in one pile. Priya files each mail into a folder, drafts a reply for the ones that need one, and loses half an hour — first thing, every morning.

## 2 · The trigger

This recipe runs as a **sweep**: a schedule fires every 15 minutes and the flow checks the mailbox for anything new. A sweep keeps the recipe self-contained and works against any IMAP mailbox. (Flow-Like also has a dedicated Email event type that invokes a flow per incoming mail — the Events course covers when that's the better fit.)

The sweep interval is a promise, not a race: 15 minutes is faster than Priya ever was, and slow enough that a run comfortably finishes before the next begins.

## 3 · The flow

Type "mail" into the node search and the catalog answers with everything this recipe needs:

@NodeCatalog

That's the actual support board with the node menu open — "mail" typed into the search, and the catalog listing mail actions from sending to inbox handling, all one search away.

The triage chain, node by node:

1. **IMAP Connect** — opens the session to your mail server (host, port, credentials) and caches it for the rest of the run.
2. **IMAP Inbox** — wraps the mailbox you're sweeping.
3. **List Mails** — returns mail references for the mailbox, with a Filter pin. Set it to `UNSEEN`: only mail nobody — human or flow — has handled yet.
4. **For Each** — loop over the references.
5. **Fetch Mail** — pulls the full message; **Email → Content** and **Email → Headers** expose subject, bodies, and addresses as pins.
6. **Branch** — the classification. Start dumb: subject contains "invoice" → invoices; contains "bug" or "error" → bugs. Dumb-but-transparent beats clever-but-opaque in week one.
7. **Move Mail to Mailbox** — the act: file the mail into `Invoices` or `Bugs`.
8. **Mark Mail as Seen** — the idempotency flag, for anything that stays in the inbox.
9. **Create Draft** — for mail that needs an answer, append a reply draft to the Drafts folder. A human still presses send.

And there's the answer to the opening riddle: the broken version never marked anything as seen and never moved anything out. Every sweep re-listed the same "new" mail and cheerfully re-answered it. The `UNSEEN` filter is only a dedupe gate if every handled mail *exits* the filter — moved out of the swept mailbox, or marked seen — and only after the handling actually succeeded.

## 4 · Guardrails

- **Draft, don't send.** Week one, the act step is Create Draft. A wrong draft costs nothing; a wrong send costs trust. When SMTP Connect and Send Mail join the recipe later, they start with the safest category only — invoice acknowledgments.
- **Exit the filter last.** Move or mark seen *after* the classification and drafting succeeded. If the run dies halfway, the mail is still unseen next sweep — a free retry, courtesy of ordering.
- **Never answer robots.** Out-of-office replies and delivery notices are automated senders; answering them starts conversations between machines. Route anything that smells automated straight to file-and-mark-seen, no draft.

## 5 · Keep it

Once a week, glance at the run history: sweeps should be green and boring. Once a month, sample the folders — five mails from each — and check the filing. When the misfile rate stays near zero, promote one category from draft to send. That's how this recipe grows up: one reversible step at a time.

**Recap**

- Sweep with List Mails on `UNSEEN`; every handled mail must exit that filter.
- Exit the filter only after handling succeeds — ordering is your free retry.
- Drafts before sends; robots get filed, never answered.
