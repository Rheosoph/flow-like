A FlowScript function becomes a callable function layer. Its parameters and named results are the boundary pins callers depend on.

```ts
function normalize(input: string): (label: string) {
    const trimmed = string::trim({ string: input })
    return string::toUpper({ string: trimmed })
}

eventsSimple testNormalize() {
    const result = normalize({ input: "  crate b  " })
    log::info({ message: result, toast: false })
}
```

Try this on a separate empty practice board after checking diagnostics. Expect `CRATE B` from `testNormalize`.

A function returning values uses one final return outside branches and loops. If branches calculate a result, assign a local mutable value in those branches and return it afterward. Event handlers have their own return rules; do not assume JavaScript's rules apply everywhere.

**Check:** the generated function's input and output match the source signature, and its caller supplies the input. For branching and collection behavior, use **Control Flow** before expanding the function.

[Function rules](https://docs.flow-like.com/studio/flowscript/#functions-and-handlers)
