---
title: Overview
description: Creating Flow in Flow-Like
sidebar:
  order: 10
---

**Flow Like Studio** is a *no-code* development environment where you build your *Flows* for automation.

Important components of the Studio environment are:
- [**Nodes**](/studio/nodes/) that you can select from the [**Node Catalog**](/nodes/overview/),
- [**Edges/Wires**](/studio/connecting/) for **Execution** and **Data** transmission between *nodes*,
- a **Canvas** where you can place your nodes and *build* your *flows*,
- [**Layers**](/studio/layers/) that allow you to collapse and define higher-order *nodes*,
- [**Variables**](/studio/variables/) available at the *Board* level to store and access information at runtime,
- [**Run History**](/studio/logging/) to inspect previous flow executions,
- [**Logs**](/studio/logging/) stored for every *run* for inspection and tracing,
- [**FlowPilot**](/studio/flowpilot/) AI assistant for building flows with natural language.

A *Flow* represents a *process* and consists of one or more *Nodes*. Nodes are linked through *Edges* (or *Wires*) for *Execution* and *Data*.

![A screenshot of Flow-Like Studio - a no-code environment to create workflow automations](../../../assets/FlowLikeStudio.webp)

Within *Apps*, flows are managed in [Boards](/apps/boards/). You can add as many *Flows* within one *Board* as you like, giving you fine-grained control over how to organize your projects:
```text
Flow Like Desktop
├── App1
│   ├── Storage and Databases
│   ├── Events
│   └── Boards
│       ├── Board1
│       │   ├── Flow1
│       │   └── Flow2
│       └── Board2
└── App2
```

*Flows* can access and modify [file storage](/apps/storage/) and [databases](/apps/data-studio/) within their app. Similarily, [events](/apps/events) can be configured for all *Flows* at the app level.
