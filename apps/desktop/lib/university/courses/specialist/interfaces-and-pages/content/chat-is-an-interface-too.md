Half of support's day is conversation, and the `/chat` route already answers with the Support assistant. Out of the box it genuinely works — type, send, the flow replies. Two problems remain: it looks like every default chat on earth, and legal has one non-negotiable — people must know they're talking to AI.

@ChatUI

That's what the team sees at `/chat` today: a welcome panel ("Support assistant — a welcoming chat interface for customer questions"), a composer with attach and send controls, three suggestion chips (*Where is my order?*, *Help me update my subscription*, *I need to speak with support*), and — bottom center — the disclosure pill: "AI assistant — responses may need human review."

> **Predict first:** of those pieces, which can you restyle, which can you reword, and which can you not remove at all?

## 1 · A chat event, not a page

You don't build this surface — you configure it. A **Chat UI** Event invokes a flow through the built-in chat interface: it passes the chat context, such as message history, to the flow's Chat Event node, and can also accept file attachments, tools, and default prompts. No canvas, no hierarchy — the interface comes with the event type.

## 2 · Make it yours

The event configuration gives you four levers:

- **Background image**, from app storage or an external URL. A storage-backed image keeps its stable path and requests a temporary signed URL when the chat loads — signed credentials are never saved in the event.
- **20 theme presets**, from professional (Executive, Swiss, Editorial) to personality (Typewriter, Neon Grid, Cyberpunk, animated Aurora Glass). Presets follow the app's current light or dark mode.
- **Custom CSS**, edited in the built-in Monaco editor.
- **AI disclosure text** — visible on every chat, and worth writing well.

The preset–CSS relationship is worth internalizing: selecting a theme copies its *complete* CSS into the editor. Change any part of it and the theme selector immediately flips to **Custom**. Select the preset again and the editor contents are replaced with that preset's canonical source — your edits are gone. Presets are starting points, not layers you compose.

In the CSS, `:root` means *this chat only* — not the app, not the document:

```css
:root {
  --primary: #7c3aed;
  --fl-chat-content-width: 52rem;
  --fl-chat-message-radius: 1.25rem;
}

[data-fl-chat-message="assistant"] {
  border: 1px solid var(--border);
}
```

Chat-specific variables cover the composer, message bubbles, the welcome surface, and the background overlay (`--fl-chat-user-message-background`, `--fl-chat-surface-background`, …), and stable selectors like `[data-fl-chat-composer]` and `[data-fl-chat-suggestion]` give you precise hooks for targeted styling.

## 3 · The disclosure stays

The AI disclosure is a feature, not a decoration: it stays visible so people know they're chatting with AI. You control its wording — keep it direct, or give it personality. The docs' own example: *"AI on duty: clever, quick, and occasionally confidently weird."* Match it to your team's voice; don't fight its existence.

**Watch out:** custom CSS is sanitized and scoped to this one chat. It can restyle everything you saw in the screenshot — it can't remove the disclosure, and it can't leak styles into the rest of the app.

**Recap**

- Chat UI is configured, not built: the event provides the interface and passes chat context to the flow's Chat Event node.
- Presets copy their full CSS into the editor; any edit makes it Custom, and reselecting a preset replaces your changes.
- The disclosure text is editable but the disclosure stays visible — style the chat around it.
