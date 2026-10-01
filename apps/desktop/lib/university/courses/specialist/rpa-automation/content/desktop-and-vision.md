Use a disposable text-editor window on your own workstation. Keep other confidential windows out of view. This exercise reads and captures; it does not submit actions to another system.

1. **Start Automation Session**, then locate the practice window with **Find Window By Title** and **Focus Window**.
2. Inspect **Get Accessibility Tree**. Find a known control with **Find Accessibility Element** if the app exposes it.
3. Record the window identity and inspect **List Displays** before using coordinates.
4. Capture only the practice window or a small region. For a custom-rendered control with no usable accessibility element, use the captured control as a template for **Find Template**.
5. Move the window and repeat. If you change theme or display scaling, capture a new template and compare the match result.
6. **Stop Automation Session** on completion and failure paths.

Grant the OS capture/accessibility permissions required by the nodes on this machine. Run a small capture test before building a larger flow. Record missing permissions as environment failures rather than changing selectors at random.

Prefer accessibility metadata when it identifies the control. A template is tied to pixels and can become invalid after scaling or visual changes. If you test a click, use a harmless focus action in the disposable editor and verify the intended control received it.
