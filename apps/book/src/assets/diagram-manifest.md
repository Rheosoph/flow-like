# Book diagram manifest

The conceptual diagrams use the FlowBook print palette, Inter typography, warm neutral surfaces,
and explicit directional arrows. Text and system labels remain deterministic SVG content.

| Asset | Purpose | Primary source |
| --- | --- | --- |
| `platform-map.svg` | Separate Studio, App domain objects, Boards, runtime, and the existing estate | Introduction, Chapter 3, `SOURCE_MAP.md` |
| `value-placement.svg` | Choose state and configuration mechanisms by owner and lifetime | Chapter 11 |
| `event-surface-adapters.svg` | Distinguish caller surfaces, App Event configuration, Event nodes, and shared Functions | Chapter 13 |
| `authoring-roundtrip.svg` | Show lowering, rendering, parsing, reconciliation, guarded Apply, and Board execution | Chapter 14 and `packages/core/src/flow/ast/` |

The diagrams avoid deployment-maturity claims and do not present FlowScript as an execution
engine. A numbered Board version freezes authored graph state; credentials, data, external
systems, and package behavior retain their own lifetimes.
