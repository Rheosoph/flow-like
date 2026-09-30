FlowScript is the text representation of a Flow-Like board. It resembles TypeScript but maps calls, functions, and events to graph nodes; it is not a general JavaScript runtime.

Open a practice board's **FlowScript** workspace. Find its event and node calls. A call such as `log::info({ message: "Ready", toast: false })` names a catalog operation and its exact input pins.

- `eventsSimple` defines a Simple Event entry.
- `string::trim` is the qualified call for Trim String.
- `log::info` is Print Info.
- An object key such as `message` is an input pin name, not its translated display label.

Compare those calls with the nodes on the canvas. **Check:** you can map the event and logger in both views. Generated node anchors preserve graph identity; keep those comments when editing existing source. Removing them can turn an intended update into a deletion/recreation.

[FlowScript syntax and anchors](https://docs.flow-like.com/studio/flowscript/)
