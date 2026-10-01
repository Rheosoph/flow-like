This class needs a flow with a Chat Event and a harmless response. Chat response behavior belongs to the AI courses; here you configure its interface.

1. Create a **Chat UI** Event targeting that Chat Event node and give it a unique route.
2. Set a short welcome message and two example prompts that describe what this flow supports.
3. Choose a theme preset. Save a copy of your CSS before editing it.
4. Change one color variable in Custom CSS, save and open the route in light and dark mode.
5. Check the disclosure, composer, keyboard focus, long messages and attachment control if enabled.

Selecting a preset copies its CSS into the editor. Editing it changes the selection to Custom; reselecting a preset replaces those edits. CSS is scoped to this chat and sanitized. The AI disclosure remains visible; its wording is configurable.

Background images can come from app storage or an external URL. Use a stored image reference rather than copying an expiring signed URL into configuration.

The result is a usable chat entry point with clear scope, not just a theme preview. Submit one synthetic prompt and inspect the run through the route.
