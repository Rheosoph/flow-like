Create an empty practice Flow named **Source normalizer**. Open its FlowScript workspace and use the following source, also available as a download:

@NormalizerSource

```ts
eventsSimple normalizeLabel() {
    const trimmed = string::trim({ string: "  cedar  " })
    const upper = string::toUpper({ string: trimmed })
    log::info({ message: upper, toast: false })
}
```

Review compiler diagnostics before applying. Inspect the generated graph: Simple Event drives Print Info; Trim String feeds To Upper Case, whose value feeds Print Info's Message.

Run `normalizeLabel`. **Expected:** the run logs `CEDAR`. Change only the input literal to ` Birch `, apply the reviewed change, and run again. Expect `BIRCH`.

Use this whole-document sample only on an empty practice board. In an existing board, edit its current source and retain its anchors so unrelated nodes keep their identities.

The download supplies source, not a preconfigured App or an automatically verified exercise. The compiler and your run must establish that it works in your catalog.
