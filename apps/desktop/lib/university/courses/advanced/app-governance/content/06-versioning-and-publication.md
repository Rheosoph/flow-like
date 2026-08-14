The 2 a.m. page: production ran a half-finished draft. Nobody deployed anything — Marek had simply kept building on Latest after dinner, and the CRM sync Event was following Latest instead of a pinned version. Every save was a silent production release.

> **Predict first:** in Flow-Like, what's the difference between saving a Flow and releasing one?

## 1 · Latest moves; versions don't

@BoardVersions

That's the Manage Board dialog for Customer Support Automation. Below the name, description, stage, and log level sits the Version selector — currently **Latest (1.0.0)** — and a **Create Version** button whose menu offers three version types: **Major**, **Minor**, **Patch**. (One incidental line rewards lesson-1 graduates: "Offline projects only support local execution.")

Create Version saves the current draft as an immutable `major.minor.patch` snapshot and moves the editable draft forward. **Latest** is always the mutable working state; numbered snapshots are read-only, forever. Pick patch for a compatible correction, minor for a compatible capability, major when consumers may need migration — Flow-Like doesn't infer semantic impact, the release owner decides it.

An Event can follow Latest or pin a numbered version. Latest is for intentional development entry points; production APIs, schedules, Pages, and Chat surfaces get pinned. That's the prediction resolved: *saving* changes Latest; *releasing* is cutting a snapshot and deliberately moving a pinned entry point to it. Marek only ever did the first — and one more thing: the App's free-form metadata version is a label for a tested collection, not a snapshot of anything.

## 2 · The release, worked

The Copilot's production CRM sync is pinned to 1.4.2 and a compatible fix is ready. The release, start to finish:

1. Fix on Latest — the pinned production Event doesn't notice.
2. Review and test the change, then Create Version → Patch: 1.4.3 exists, immutable.
3. Move the *staging* Event to 1.4.3 and verify the complete path — invocation, permissions, data, telemetry.
4. Pin the production Event to 1.4.3; verify again, including denied paths.
5. Keep 1.4.2 selectable until rollback stops being plausible.

Rollback is the same machinery in reverse: re-pin 1.4.2 (it already worked), debug on Latest, cut 1.4.4, re-pin. Nobody "fixes" 1.4.3 — snapshots are immutable, and rediscovering that mid-incident wastes the first ten minutes of the response.

## 3 · Publication is a request, not a toggle

Lesson 2's footnote comes due here: public transitions require central review (1–3 days). Requesting a public target creates a **publication request** with an activity log. It moves through pending, on hold, accepted, or rejected — and an App has only one active pending or on-hold request at a time. Reviewer feedback gets resolved on that request; a duplicate submission is rejected.

Give reviewers a package that answers their questions before they ask: purpose and audience, public routes and Events with their authentication and data scope, the pinned Flow and interface versions, test results with known limitations and the rollback plan, the role and default-role review, a credential summary (owners and mechanisms — never values), and monitoring and incident contacts.

When the platform's AI Act feature is enabled, one more gate applies: a request toward a public target requires the App's latest assessment to exist and be neither Draft nor Blocked. A missing, draft, or blocked assessment stops the request. It's a publication gate — not evidence that the system is technically safe.

**Watch out:** approval changes visibility and nothing else. It doesn't snapshot, and it doesn't pin. If a production Event still follows Latest on acceptance day, you've published drift — pin first, then request.

## Recap

- Latest is mutable; numbered versions are immutable snapshots — production entry points pin them.
- Rollback re-pins a version that already worked; fixes ship as new patches.
- Publication is a reviewed request with evidence: one active request, feedback resolved in place.
