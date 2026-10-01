Build a workflow with a result you can check without an AI model.

1. Right-click the empty canvas and add **Simple Event** (`events_simple`).
2. Add **Print Info** (`log_info`).
3. Connect Simple Event's **Output** (`exec_out`) to Print Info's **Input** (`exec_in`). These diamond pins carry execution.
4. Set Print Info's **Message** (`message`) to `Hello Flow-Like` and **On Screen?** (`toast`) to `true`.
5. Start the run from the Simple Event's play control.

**Expected:** an information toast says `Hello Flow-Like`. Open **Runs**, select this run, and find the same message attributed to Print Info. If the toast disappears, the log is still your result.

Change Message to `Second run`, run again, and open the new run. It should contain the new text. The previous run remains evidence of the earlier input.

If nothing prints, check that the execution wire reaches Print Info, then check that you selected the latest run. Keep the two-node board: later courses extend it.

[Runs and logs](https://docs.flow-like.com/studio/logging/)
