Introduce a controlled bug in Normalize Label: connect Trimmed String directly to Print Info's Message, bypassing To Upper Case. If the transformations are inside a layer, change the connection to the layer's returned value there.

1. Use input `  cedar  ` and run. Expect the faulty result `cedar`.
2. Open that run and locate Print Info's message. The run can complete successfully while violating the required uppercase output.
3. Trace backward from Message. The first wrong connection should bypass To Upper Case.
4. Restore the Uppercase String connection, keeping the input unchanged.
5. Run again and verify `CEDAR`. Then rerun all four cases from the build lesson.

**Done when:** every case has its expected result and you can identify the changed connection. Preserve the failed run as a comparison point.

For an AI or external-service workflow, identical inputs can still produce different responses. This exercise deliberately uses deterministic string nodes, so its exact-output checks are meaningful.

[Debugging reference](https://docs.flow-like.com/studio/logging/)
