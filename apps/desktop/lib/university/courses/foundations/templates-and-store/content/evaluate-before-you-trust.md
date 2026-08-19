Your Explore search turns up **Inbox Wizard**: thousands of installs, a glowing rating, a verification shield. Priya's cursor is already hovering over Install. Then you read the fine print: it declares HTTP access to *any* host, plus user-scoped storage. For a package that sorts email? Maybe fine. Maybe a data exfiltration bot with good UX.

> **Predict first:** Which tab tells you more about whether to trust this package — **Reviews** or **Permissions**?

## 1 · Read the card

The card itself already carries signals: current version, category, install count, rating, price, and visibility. A shield marks a package that completed the registry's verification process. When you're browsing, you can sort by Most Downloads, Relevance, Name, Recently Updated, or Newest, and a **Verified** filter limits results to reviewed packages.

Install count and rating tell you the package is *popular*. They don't tell you it's *appropriate* for a client app that handles support mail. For that, open the detail page.

## 2 · Open the tabs

A package detail page gives you five views:

- **Overview** — description, README, author, links, and publication info supplied by the maintainer.
- **Nodes** — what the package actually exports. If the README promises twelve nodes and the tab shows two, believe the tab.
- **Permissions** — declared resource limits and host capabilities.
- **Versions** — available, installed, yanked, disabled, or in-review versions.
- **Reviews** — what other users experienced.

Permissions are the tab your prediction should have picked. Declarations can include network access (optionally constrained to specific hosts and protocol families), scoped storage, OAuth scopes, runtime variables, cache, streaming, A2UI, or model access. The evaluation rule is one sentence: **compare the declaration with what the nodes are supposed to do.** A mail parser asking for mail-server access is coherent. A date formatter asking for network access to any host is a question you must be able to answer before installing.

> **Watch out:** The verified shield records a review state — it is not a security warranty, and it doesn't promise that every future version behaves identically. Review the package, version, permissions, author, and source links yourself.

## 3 · The deepest test drive: fork it

Packages extend your catalog, but sometimes the thing worth evaluating is a whole community app — someone else's support kit, say. For that, **forking** exists: you get your own copy to open, inspect, and modify, while the original app stays untouched. Nothing you do in a fork flows back upstream, so you can experiment as hard as you like.

Two things to know before your first fork:

- **What the fork contains is not your call.** The app's owner defines a fork policy; the fork dialog shows you what you'll receive. If a fork arrives with flows and widgets but no storage files, that's the owner's policy at work, not a bug. (In lesson 4 you'll sit in the owner's chair and set this policy yourself.)
- **You choose where the fork lands**: your online account as a private cloud copy, or this device as a local offline copy.

A fork is the honest evaluation: you see the real graphs, the real structure, and how much work adapting it would take — before you bet a client project on it.

## Recap

- Popularity signals (installs, ratings) say *used*, not *appropriate* — the Permissions and Nodes tabs say appropriate.
- Verification is a review state, not a warranty.
- Forking gives you a private copy for a real evaluation; the owner's fork policy decides what's inside.
