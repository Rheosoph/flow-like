A flow can complete successfully and still return the wrong result. This lab uses text normalization so replay has no external effects.

@PracticeFiles

## Build the intentional fault

1. Add **Simple Event** and **Print Info** (`log_info`); connect their execution pins.
2. Add **Trim String** and **To Upper Case** from the String catalog. Set Trim String's String input to `  Mixed CASE  `.
3. Connect Trim String's result to To Upper Case's String input, then its result to Print Info's Message. These transformation nodes use data pins, so the value is evaluated when the log needs it.
4. Run and select the result in **Runs**. Expect a green run with `MIXED CASE` in the log. The fixture's required result is `mixed case`.

Record the event/entry, executed version, input, output and duration. You now have a reproducible mismatch rather than a suspicion.

Run again with exactly the same fixture. If you use a historical run's Re-Run action, inspect which flow/version you are about to invoke. Replay uses recorded input but can still perform the flow's side effects. This lab logs only synthetic text; real sending or writing flows need a safe replay target.

If the log appears empty, inspect its level filters and the board's configured log level. Absence from the current view is not proof that the node never ran.
