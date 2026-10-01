Create **Check label**, using Simple Event, **Try Catch** (`rpa_try_catch`), **Assert** (`flow_assert`), and two Print Info nodes.

1. Simple Event Output → Try Catch **▶** (`exec_in`).
2. Try Catch **Try** (`exec_try`) → Assert **Input** (`exec_in`).
3. Set Assert **Condition** (`condition`) to false, **Label** (`label`) to `label-present`, and **Details** (`details`) to `synthetic missing label`.
4. Try Catch **Catch** (`exec_catch`) → the first logger's Input; set Message to `REJECTED`.
5. Try Catch **Success** (`exec_success`) → the other logger's Input; set Message to `ACCEPTED`.
6. Leave Try Catch Error Occurred false and Error Message empty. Run the event.

**Expected:** Assert emits an `ASSERT_FAIL label-present` error and Try Catch routes to `REJECTED`, with no `ACCEPTED` message. The handled failure remains visible in the logs.

Set Assert Condition true and rerun. Expect `ASSERT_OK label-present` and `ACCEPTED`.

Try Catch also exposes **Message** (`message`) for inspecting the captured error. Use synthetic inputs here: logging a raw production error can reveal data. Each operation's error contract matters; ordinary For Each logs item failures and continues, so Done alone does not prove every item succeeded.
