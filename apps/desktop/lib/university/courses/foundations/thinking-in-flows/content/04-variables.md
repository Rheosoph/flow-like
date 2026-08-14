Riko's ticket needs her name in the greeting, a confidence score for the draft, and a flag that says whether angry kettle owners get escalated to a human. Three values, needed in different corners of the board, some changing mid-run. Dragging one wire across the whole canvas three times is technically possible — and visually a crime scene. This is what variables are for.

@FlowVariables

That's the variables panel, opened from the (x) button in the top toolbar. The support board defines four: **Confidence** (green dot — Float), **Customer Name** (pink — String), **Escalation Enabled** (red — Boolean), and **Routing Tags** (violet, with a grid icon marking its collection shape). Each row also carries an eye toggle — Customer Name and Escalation Enabled show an open eye, Confidence and Routing Tags a crossed-out one. Hold that thought; the eye comes up in step 3.

## 1 · Wire or variable?

Use a direct wire when one producer feeds a nearby consumer. Reach for a variable when several regions need the value, when it changes during the run, or when a caller or device must supply it. And name it for its meaning — `Escalation Enabled` beats `bool_3` in every code review you'll ever have.

Variables are per-run, in-memory state: each invocation gets its own copy, so two simultaneous runs never see each other's values, and nothing lingers after a run ends.

## 2 · Get, Set, and who wins

Drag a variable from the panel onto the canvas and choose **Get** (read it) or **Set** (write it). A Get returns the current value; a Set updates it and belongs on the white execution path, like any other consequential act.

> **Predict first:** a Get Confidence node returns 0.2, yet a Set Confidence holding 0.9 sits directly beside it on the canvas. Broken?

No — unexecuted. What a Get returns is decided by the last Set that *execution actually reached* earlier in the run. Proximity on screen counts for nothing (lesson 1's rule, back again). When a value surprises you, hunt down every Set on the executed path, in order — not the one nearest the Get.

## 3 · Type, shape, and the three switches

@SetVariableType

Editing a variable opens this panel: a name, an optional category (use `/` for nested folders), and the type picker — here Confidence is a **Float**, chosen from Boolean, Date, Float, Integer, Generic, PathBuf, String, Struct, and Byte. The dropdown beside it selects the shape: a **Single** value, or a collection (array, set, or map). Type and shape become part of the graph's contract — generated Get and Set nodes expose matching pins.

Then three switches that answer three different questions:

- **Exposed** — *who may supply it?* An exposed variable (the open eye in the panel) can be filled by app configuration or a compatible invocation. Expose real inputs, not internal scratch state.
- **Secret** — *how is it handled?* The value is masked in editors and treated as sensitive.
- **Runtime Configured** — *where does it live?* Not in the flow definition at all: each user and device supplies its own value via the app's Runtime Variables screen.

For a credential — say the mail provider token this board needs — combine **Secret + Runtime Configured** and enter the real value only in Runtime Variables. The token then never sits inside the flow, never syncs to teammates, and never rides along in an export. This is the course's one security sermon; later lessons will just point back here. A masked default is *not* a substitute: masking changes the display, but the token would still be saved into the flow itself.

## Recap

- Variables are typed, named, per-run state; each run starts fresh.
- A Get reflects the last Set execution reached — order on the path, not distance on screen.
- Exposed = who supplies it, Secret = how it's shown, Runtime Configured = where it's stored. Credentials take the last two.

Next: the board looks this tidy because half of it is folded away — time to open the layers.
