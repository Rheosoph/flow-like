Create a practice Flow named **Dispatch decision**.

1. Add Simple Event (`events_simple`), Branch (`control_branch`), and two Print Info (`log_info`) nodes.
2. Connect Simple Event **Output** (`exec_out`) to Branch **Input** (`exec_in`).
3. Connect Branch **True** (`true`) to the first logger's Input. Set its Message to `SEND`.
4. Connect Branch **False** (`false`) to the other logger's Input. Set its Message to `HOLD`.
5. Set Branch **Condition** (`condition`) to true and run; then set it to false and run again.

**Expected:** the true run logs only `SEND`; the false run logs only `HOLD`. Keep On Screen? false and inspect the run logs. The two labels stand in for actions; this practice board never sends anything.

A condition chooses one execution path. When replacing a literal with a computed Boolean, wire that value to Condition and repeat both cases. A named approval layer or a favorable model response is not a substitute for an explicit, implemented decision.
