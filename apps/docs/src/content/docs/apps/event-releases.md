---
title: Release and roll back an Event
description: Test candidate Flow versions, split traffic, and restore a known Event configuration.
---

An Event is a live entry point. Releasing it means choosing which Flow or Page
serves its callers and checking the result. A **Flow version** captures a graph
and its owned Pages. An **Event version** records the configuration of the
entry point that uses it.

This guide applies when your online App and client expose **Quality**,
**Canary**, and **History** in the Event editor. Their availability depends on
the client, hub, Event type, and your permissions. Enable
[Developer Mode](/start/developer-mode/) to find **Events**. If a section is
absent or reports that it is unavailable, check that the client and hub support
the operation before planning a release around it.

## Prepare a candidate

1. Open the Event and record its primary Flow, event node, version, and route.
2. Leave the live Event pinned while you edit the Flow's Latest draft.
3. Run representative inputs in Studio, then create a numbered
   [Flow version](/studio/versioning/).
4. Check that the Event node, inputs, and required runtime-variable overrides
   exist in the candidate. For a Page, also check its lifecycle workflows and
   navigation in Preview.

A Flow snapshot includes its owned Pages. Tables, stored files, credentials,
App styles, and reusable Widget definitions remain separate resources. Include
them in your release checks when behavior depends on them.

## Validate with Quality

**Quality** replays recorded inputs and authored tests against a candidate.
Reading the suite needs Event read access. Reading recorded inputs and case
logs also needs log access; running a suite requires Event execution and log
access. Editing its configuration needs Event write access.

Page Events and ontology actions do not support this regression suite. Page
payloads belong to a page session; use an interface test for that path.

1. Open **Quality → Suite configuration** and save a suite.
2. Review and acknowledge the live-side-effect notice. Storage writes and WASM
   execution are isolated for suite runs, but native outbound calls can still
   send mail, post webhooks, or change external systems. Use test destinations
   for such calls before running the suite.
3. Under **Recorded inputs**, select representative runs and choose **Add to
   regression set**. Inspect the redacted input and its baseline expectation.
4. Include passing and failing cases. A stored failing baseline helps identify
   both a new regression and a corrected failure.
5. Run the suite against the numbered candidate and inspect each case's verdict,
   failed assertions, and logs.

The comparison uses graded verdicts, not exact output equality. Add
[Assert](/nodes/utils/testing/flow-assert/) nodes for semantic expectations.
Event nodes whose names start with `test` are discovered as authored tests and
run with the suite. Info-level assertion markers must survive the Flow's log
level for the runner to justify a passing verdict.

The promotion gate can be **Off**, **Warn**, or **Block**. It is consulted for a
candidate version the suite has run against. A run against **Draft head** does
not feed that gate. Inspect the candidate's result instead of assuming an
untested version inherits another version's verdict.

Automatic schedules and publication-triggered runs are configured in the suite.
Fixtures originally using caller OAuth tokens cannot supply those tokens to a
scheduled replay; remove or redesign those fixtures before scheduling.

## Send a share of traffic to the candidate

Canary releases apply to cloud execution. Local runs always use the primary
target, so they cannot verify a cloud traffic split.

1. Open **Canary** and choose **Add variant**.
2. Give it a distinct name and select the candidate Flow/node/version or Page.
3. Review inherited variables and any overrides. Variants use the Event's sink
   identity and credentials.
4. Set a small initial traffic share and save. Weight zero receives no weighted
   traffic and is useful when testing through an explicit variant pin.
5. Observe the candidate's traffic, errors, latency, and test results alongside
   the primary before increasing its share.

Traffic-weight changes apply immediately and do not create an Event version.
A Page canary assigns a viewer when the Page loads; an open page session keeps
that assignment until reload. Pages do not support shadow mode. For REST and
MCP, check each variant's setup health: a variant without successful inbound
registration receives no inbound traffic.

Use **Explain assignment** to inspect which variant serves a supplied split key.
A small traffic share can produce no observations in a short window. Zero
recorded traffic is not evidence that the candidate passed real requests.

## Promote or abort

**Promote** makes the variant's target the primary, removes that variant, and
creates a new Event version. Review the confirmation and any Quality gate
result before accepting it. Test the public entry point after promotion.

**Abort variant** removes the variant and returns its traffic to the primary
immediately. Its inbound registrations are removed. Existing Page sessions
retain their assignment until reload; check fresh sessions when verifying the
change.

## Restore an earlier Event version

1. Open **History** and select the last known working Event version. Inspect
   **Runs**, **Nodes**, and **Compare** as needed.
2. Choose **Restore this version** and read the restore plan.
3. Resolve blockers such as a missing Flow or event node. An archived run can
   remain visible even when its snapshot is no longer restorable.
4. Decide whether to restore the old route. By default the current route and
   default-route flag are kept. Review whether to retain the snapshot's canary.
5. Review secret-recovery warnings. Unrecoverable secret values stay blank and
   must be re-entered before the Event can operate correctly.
6. Confirm restoration, check endpoint setup status, and test the complete
   invocation path with a representative input.

Restoration creates a new Event version whose content follows the selected
snapshot. The previous live version remains in history. Restoring Event
configuration does not reverse data changes or external side effects caused by
earlier runs.

Keep debugging on the Flow draft, then create and test a new candidate. For
individual run investigation, see [Logging and debugging](/studio/logging/).
