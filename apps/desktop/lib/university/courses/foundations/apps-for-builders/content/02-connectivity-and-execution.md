Make three decisions explicitly.

| Decision | Meaning |
| --- | --- |
| Offline or online App | Where the project is stored and how users reach it |
| Local, Remote, or Hybrid Flow | Which execution hosts are allowed |
| Event location | Which runner answers that configured invocation |

An offline App runs locally. An online App can still require Desktop execution. Hybrid chooses a host per invocation; it does not divide one run across machines. Device-only nodes, including nodes hidden inside layers, can force the whole run to stay local. A local run can still call a cloud API.

For Equipment Register, write two deployment rows:

1. **Manual laptop check:** offline App, local Flow, Desktop available when invoked.
2. **Nightly report with laptops closed:** online App, remote-compatible Flow, remote Event, backend credentials and data available.

**Check:** each row names the machine that must be available. Do not promise unattended execution from a laptop that will be off.

Moving an offline project online creates a separate online copy. Review its credentials, paths, Events, and access; device runtime secrets do not become server credentials.

[Execution placement](https://docs.flow-like.com/studio/local-execution/)
