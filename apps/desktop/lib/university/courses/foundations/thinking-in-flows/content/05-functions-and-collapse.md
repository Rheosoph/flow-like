The support board shows four nodes on its main path. The actual work involves more — but you'd never know from a zoomed-out glance, and that's deliberate. Two of those four "nodes" are folded graphs. This lesson teaches the fold.

## 1 · Collapse a selection

@CollapsedLayer

In this shot both amber pure nodes are selected (orange outlines), and a toolbar hovers over the selection with a **Collapse** tooltip on its first button. Click it, and the selection folds into a **layer**: one node on the outer board whose boundary pins mirror every wire that crossed the selection edge. Rename it immediately — `Prepare Support Reply` tells a story, `Layer 2` tells on you.

Collapsing changes presentation, not behavior. The inner nodes still follow every execution and data rule from lessons 2 and 3; they're just behind a door now.

## 2 · Inside the door

@InsideLayers

Open Prepare Support Reply and this is what you find: a red **Start** node hands the Message value in, Normalize Request feeds Draft Helpful Reply, and a **Return** node carries the Reply back out. A green comment up top spells out the intent ("Normalize the request, then draft a helpful reply"), and the layer's name floats as a watermark in the corner so you always know where you are. The highlighted toolbar button takes you back up.

Start and Return are the layer's contract made visible: their pins *are* the boundary pins the outer board sees. Which makes every boundary edit an interface change. Rename Message, add a Tone input, retype Reply — then check every outer wire into the layer, confirm the inner Start-to-Return path is still connected, and give any new input a source or a sensible default. Skip that audit and the outer board finds out at run time.

## 3 · Prototype with placeholders

Here's the sneaky one: **Human Review does nothing.** Its comment admits it — "Prototype a future review step before implementing its internals." It's an intentionally empty layer: a named boundary with agreed pins and no implementation, holding a seat in the architecture so the team can review the shape of the flow before building the middle.

@PlaceholdersForPrototyping

The catalog makes this a first-class move — note the highlighted **Placeholder** quick action above the search box, next to Comment and Event. Drop a placeholder, name the responsibility, define the pins, wire it in. Design review on Tuesday, implementation whenever. Just keep unfinished layers visibly named as such: a convincing top-level graph is not a runnable feature.

## 4 · From layer to function

When folded logic deserves *reuse* — called from several places on the board rather than sitting inline once — convert the layer into a **function**. It keeps its nodes and boundary pins, gains a callable signature, and a **Call Function** node takes its place; the panel from lesson 4 lists functions right below variables.

Two rules gate the conversion. A function that participates in execution needs **exactly one execution entry** — no entry can't be called, two entries can't decide where to begin, so merge first. And boundary pin names must be unique, because they become the signature callers read. Convert when reuse is real; a two-node layer with a vague name adds a door with nothing behind it.

> **Watch out:** collapsing hides nodes from your eyes, not from the runtime. A local-only node inside a layer still makes the whole flow local-only, and an inner failure is still a real failure. Lesson 7 comes back to this.

## Recap

- Collapse folds a selection behind one named layer node; wires that crossed the edge become typed boundary pins.
- Start and Return are the contract — treat every boundary edit as an interface change and re-check the callers.
- Empty placeholder layers prototype architecture; functions are layers with a callable signature and exactly one execution entry.

Next module: enough reading the reference board — you build one.
