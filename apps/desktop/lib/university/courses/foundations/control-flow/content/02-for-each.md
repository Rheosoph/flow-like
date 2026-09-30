Create a second Flow, **Inspect labels**. Add a String variable **Labels**, shape **Array**, default `["cedar", "birch", "ash"]`.

| Connection | Purpose |
| --- | --- |
| Simple Event Output → For Each Input | Start the iteration |
| Get Labels value → For Each Array (`array`) | Supply the collection |
| For Each For Each Element (`exec_out`) → Print Info Input | Run once for each item |
| For Each Value (`value`) → that logger's Message | Print the current item |
| For Each Done (`done`) → a second Print Info Input | Continue after iteration |

Set the second logger's Message to `DONE`. **For Each** has node ID `control_for_each`; its **Index** (`index`) starts at zero.

Run the default list. Expect `cedar`, `birch`, `ash`, then `DONE`. Change Labels to an empty array and run again: expect only `DONE`.

@CollectionCases

Use the supplied synthetic cases to repeat the exercise. This sequential loop is appropriate when order matters. Parallel work needs separate reasoning about ordering, shared state, and concurrency limits; do not assume it behaves identically.
