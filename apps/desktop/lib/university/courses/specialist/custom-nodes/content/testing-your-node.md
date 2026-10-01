Run the supplied tests. They check serialized identity, every option combination, empty/default inputs, changed and the activated execution pin. Inspect a failure before editing an expectation.

## Add one regression case

Add an input with a trailing newline and mixed case. Specify the exact output and changed flag first. Run the test, then build again.

A mock host can prove the logic you exercised. It does not reproduce every capability check, resource limit or packaging constraint. Keep these checks separate:

1. **Unit test:** output values, defaults and execution branch.
2. **Component build:** valid artifact from the reviewed source.
3. **Local runtime:** package appears, wires correctly and executes in a real board.
4. **Saved board:** save, close and reopen the scratch board; run the same input again.

For an update, test a board saved with the prior node contract. Creating only a fresh board cannot reveal broken stored pin names. Compare capability changes and app package-version settings before rolling out an update.

Record the SDK/template version with your evidence. SDK behavior such as success()/finish() is a contract you should verify against that version rather than copy from an old lesson.
