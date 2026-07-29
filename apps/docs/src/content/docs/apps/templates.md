---
title: Templates
description: Reuse Flows with Templates
sidebar:
  order: 90
---

**Flow Templates** are reusable, versioned snapshots of individual Flows. Use
them to share a proven graph, create another Flow from the same blueprint, or
preserve a reusable starting point without publishing an entire App.

## Create a Template

1. Open the App's **Flow Templates** workspace.
2. Select **Create Template**.
3. Enter a name and description.
4. Choose the source Flow.
5. Choose a numbered Flow version or **Latest**.
6. Create the Template.

Choosing a numbered version captures an immutable Flow snapshot. Choosing
**Latest** captures the current draft at the time the Template is created; the
Template does not keep following later edits.

## Manage Template versions

Open a Template to review its description, source information, and available
versions. You can import a newer Flow snapshot as another Template version
without replacing the older one. This lets consumers deliberately choose the
blueprint they want.

Template metadata helps people evaluate the blueprint, but the executable
content comes from the captured Flow version. Test a source Flow before adding
it as a Template version.

## Templates versus Apps

| Share this | Use |
| --- | --- |
| **Flow Template** | One reusable graph and its versioned snapshots |
| **App** | A complete project boundary, including its Flows, Events, Pages, storage, data model, and access settings |

A Template is not a backup of its source App and does not include the App's
runtime variables, credentials, Events, Pages, storage, or Data Studio data.

See [Versioning](/studio/versioning/) for the relationship between the editable
Flow draft, immutable Flow versions, Event pins, and Template snapshots.
