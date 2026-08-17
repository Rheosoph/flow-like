The last ten minutes of Dana's ritual never touch a browser. Shipping labels come out of the Kestrel label client — a native desktop app last redesigned around the time USB sticks were exciting. There's no DOM in there, no selectors, nothing for Wait For Selector to wait for. To your flow, it's a window full of pixels. So how does a flow find the Export button?

> **Predict first:** without a DOM, what can a flow actually "see" inside a native app?

## 1 · Surface: session, window, display

Desktop automation begins with **Start Automation Session** and ends with **Stop Automation Session** — and "ends" means on the success path *and* the failure path, exactly like closing the browser last lesson. Keeping the session explicit makes resource ownership and cleanup visible right on the board.

Then resolve *where* you're working. **Launch Application** starts the label client if it isn't running; **Find Window By Title** locates its window; **Focus Window** brings it to the front. If absolute coordinates will be involved anywhere downstream, resolve the display first with **List Displays** or **Get Primary Display** — "the screen" is an ambiguous address on a two-monitor desk like Dana's.

## 2 · Target: accessibility first, templates second

Native apps expose accessibility metadata — the same structure screen readers use. **Get Accessibility Tree** shows you the controls the app declares; **Find Accessibility Element** locates one to act on. This is the preferred targeting method for native controls, because it can stay stable when the window moves or the display scaling changes.

The label client, though, is custom-rendered, and its accessibility tree is nearly empty. That's what Vision is for. Crop a small screenshot of the Export button and **Find Template** locates it on screen; **Click Template** finds and clicks in one step; **Wait For Template** holds until the "Export complete" dialog shows up. Three hygiene rules keep templates honest:

- Crop tightly around a distinctive, stable control — not a quarter of the window.
- Search a captured region (**Screenshot Region**) instead of the whole display; a smaller search means fewer false matches.
- Recreate templates when the app's theme, display scaling, or visual design changes. A template is a photograph, and photographs go out of date.

## 3 · Interact: hands on mouse and keyboard

With a target resolved, the Computer nodes do the touching: **Mouse Click** and **Mouse Double Click** at the resolved position, **Natural Mouse Move** when the cursor's path matters, **Type Text** and **Key Press** for the keyboard. The clipboard nodes (**Get Clipboard Text**, **Set Clipboard Text**) are the unglamorous heroes of desktop work — the label client's own "copy tracking numbers" button plus Get Clipboard Text beats scraping pixels every time. **Wait** exists for deliberate pauses between interactions; it's a pause, not a readiness signal.

## 4 · Verify: prove the label exists

After the export, assert the outcome: **Assert Template Exists** on the confirmation dialog, or **Capture Window** to keep visual evidence of the final state. And before any of this runs unattended, deal with the environment. Depending on the nodes used, the operating system will demand permission for screen capture, accessibility APIs, and mouse and keyboard control — and the prompts vary by OS. Grant only what the automation needs. Run a small capture-and-input test on the actual ops workstation before building the full flow, and run it again after operating-system, application, theme, or display changes.

> **Watch out:** display scaling is part of your environment. A template captured at 100% scaling may not match at 125%. Test at the scaling production uses.

## Recap

- Start and Stop Automation Session frame every desktop automation, on both exits.
- Accessibility elements first; templates are the fallback — crop tight, search a region, recreate on change.
- Permissions and display scaling are environment: verify them on the real machine, not just yours.
