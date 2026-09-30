Extend the two-node board from Welcome so it removes surrounding spaces before printing.

1. Add **Trim String** (`string_trim`). Set its **String** (`string`) input to `  cedar  `, including the spaces.
2. Connect **Trimmed String** (`trimmed_string`) to Print Info's **Message** (`message`). This round-pin connection carries a value.
3. Keep Simple Event **Output** connected to Print Info **Input**. Run the event.

**Expected:** the log contains `cedar`. Trim String has no execution pins: Print Info requests its output through the data wire. Print Info itself still needs the execution connection.

Remove the execution wire into Print Info and run again. It should produce no Print Info message. Restore the wire. This separates two responsibilities: execution determines when the logger runs; data determines its message.

Pins have types and directions. Studio rejects incompatible connections. A Generic pin can adapt to a connected concrete type; inspect the resolved type when a later wire is rejected. An unused pure node is not evidence that its calculation ran.

[Connections](https://docs.flow-like.com/studio/connecting/)
