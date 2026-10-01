---
title: Apps and core concepts
description: Understand Apps, Flows, Events, Pages, and the resources they own.
sidebar:
  order: 5
---

An **App** groups executable workflows with their interfaces, data, and access
settings. Create one App for a project that should be configured and shared
together, then add the Flows and entry points it needs.

![Anatomy of a Flow-Like app, connecting its Flows, experiences, data, reusable building blocks, and delivery controls](../../../assets/FlowLikeAppAnatomy.svg)

## Terms you will see

| Term | Meaning |
| --- | --- |
| **App** | The project boundary for Flows, Events, Pages, storage, data, Widgets, Templates, and access settings. |
| **Project** | The UI also uses this word for an App, including **Project Name** and **Create Project**. |
| **Flow** | Executable logic built from nodes and connections, edited visually or as [FlowScript](/studio/flowscript/). |
| **Board** | The stored graph representation of a Flow. APIs use `board_id`, and older interfaces can still use the word Board. |
| **Event node** | A start node inside a Flow, such as Simple Event or Chat Event. You can run it from Studio. |
| **Event** | A configured App entry point that selects a Flow, node, version, execution location, and interface or trigger settings. A Page-target Event opens a Page. |
| **Page** | A full A2UI interface owned by a Flow. An Event and route make it available to App users. |
| **Route** | A path such as `/support` mapped to an Event within an App. |

An Event node and a configured Event are separate objects. Several Events can
invoke the same node with different defaults or versions. A Flow version
captures its graph and owned Pages. App data, files, and independently versioned
Widgets need separate release checks; see [Versioning](/studio/versioning/).

### Four meanings of widget

| Kind | Where it belongs | Guide |
| --- | --- | --- |
| **Home widget** | A block in a Profile's Home layout, such as an App collection or image | [Customize Home](/start/home/) |
| **A2UI Widget** | A reusable declarative component tree owned by an App and placed in interfaces | [Create Widgets](/apps/widgets/) |
| **Package micro widget** | An HTML/JavaScript interface supplied by a package and hosted in an isolated frame | [Package widgets](/dev/package-widgets/overview/) |
| **Native widget** | A Flow-Like widget on the iOS Home Screen or macOS desktop, using cached content | [Native widgets](/apps/native-widgets/) |

## Choose your next task

| Goal | Start here |
| --- | --- |
| Build and verify a first workflow | [First Flow](/start/getting-started/) |
| Decide where data and execution live | [Offline vs. Online](/apps/offline-online/) |
| Organize and inspect workflows | [Flows](/apps/boards/) and [Studio](/studio/overview/) |
| Add a page, chat, or trigger | [Interfaces](/apps/a2ui/) and [Events](/apps/events/) |
| Store files or structured data | [Storage](/apps/storage/) and [Data Studio](/apps/data-studio/) |
| Configure device-specific values | [Runtime variables](/apps/runtime-variables/) |
| Share and release an App | [Sharing](/apps/share/) and [Event releases](/apps/event-releases/) |

Use Docs for current procedures, configuration, and API reference. For a
chapter-by-chapter explanation of the language and application model, start
with [FlowBook's introduction](https://book.flow-like.com/introduction/), then
return to the relevant Docs task when configuring a feature.
