Keep `request.json` beside the downloaded client. For TypeScript, also save the supplied `package.json` there. Install and run one language path in a new practice directory.

**Python**

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install flow-like
python3 invoke.py
```

**TypeScript**

```bash
npm install
npx tsx invoke.ts
```

Each client iterates the Event's Server-Sent Events response. Printed entries may include progress and protocol metadata. Locate the returned business result and compare its request ID and normalized message with `sdk-001` and `ready`; do not treat the first streamed entry as completion.

Change message to `  NEXT  ` and request ID to `sdk-002`. Run again and expect `next`. Use the run view to verify the same pinned Event handled it.

Completion: two calls have distinct IDs and matching transformed outputs. Keep only synthetic output in logs; do not print credentials. For errors, inspect authentication, target IDs and remote execution first. References: [Python](https://docs.flow-like.com/dev/sdks/python/) and [TypeScript](https://docs.flow-like.com/dev/sdks/nodejs/).
