# A2UI Frontend Builder - External AI Setup

This directory contains system prompts and knowledge files for creating Gemini Gems, ChatGPT Custom GPTs, and Claude skills that can generate A2UI JSON interfaces.

## Try It Now

**ChatGPT GPT:** [FlowLike Frontend Builder](https://chatgpt.com/g/g-6965146c7f5c81918a2501c5a860d9e3-flow-like-frontend-builder)

## Styling Guidelines

The generators use shadcn/ui theme tokens for automatic dark/light mode support:
- **Preferred:** `bg-background`, `text-foreground`, `bg-primary`, `text-muted-foreground`, etc.
- **Also allowed:** Hardcoded Tailwind colors (`bg-red-500`, `text-blue-600`) when users explicitly request specific colors

Theme tokens are preferred because they adapt to the user's color scheme, but specific color requests should be honored.

## Directory Structure

```
frontend-builder/
├── gemini/
│   └── system.md          # System prompt for Gemini Gem
├── chatgpt/
│   └── system.md          # System instructions for ChatGPT GPT
├── docs/
│   ├── components-reference.md   # Full component documentation
│   ├── bound-value-guide.md      # Data binding patterns
│   ├── styling-guide.md          # Tailwind/shadcn rules
│   └── layout-examples.md        # Complete JSON examples
└── claude/
    ├── flow-like-ui/             # Claude skill source
    └── flow-like-ui.zip          # Packaged Claude skill
```

## Output Contract

Every generated surface must use one root component named `root`:

```json
{
  "rootComponentId": "root",
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1rem"
  },
  "components": [
    {
      "id": "root",
      "style": {
        "className": "min-h-screen w-full bg-background text-foreground"
      },
      "component": {
        "type": "column",
        "children": {
          "explicitList": [
            "content"
          ]
        }
      }
    },
    {
      "id": "content",
      "style": {
        "className": "w-full max-w-3xl mx-auto p-4"
      },
      "component": {
        "type": "text",
        "content": {
          "literalString": "Hello FlowLike"
        }
      }
    }
  ],
  "dataModel": []
}
```

Rules:

- `rootComponentId` is always exactly `"root"`.
- `components` contains exactly one component with `id: "root"`.
- The root component wraps all top-level UI sections through `children`.
- All other component IDs should be unique, descriptive, and kebab-case.
- Build mobile-first and use responsive Tailwind classes so output works on phones, tablets, and desktop.

The uploaded knowledge files reinforce the same contract and include examples that follow it.

## Setup Instructions

### Gemini Gem

1. Go to [Google AI Studio](https://aistudio.google.com/) → Gems
2. Create a new Gem
3. Copy the contents of `gemini/system.md` into the **System Prompt**
4. Upload the files from `docs/` as **Knowledge** files
5. Test with prompts like "Create a login form" or "Build a dashboard with stats cards"

### ChatGPT Custom GPT

1. Go to [ChatGPT](https://chat.openai.com/) → Explore GPTs → Create
2. Copy the contents of `chatgpt/system.md` into the **Instructions**
3. Upload the files from `docs/` as **Knowledge** files
4. Enable "Code Interpreter" for JSON validation (optional)
5. Test with similar prompts

## Knowledge Files

The docs files are designed to be uploaded as knowledge/context:

| File | Purpose | Upload Priority |
|------|---------|-----------------|
| `components-reference.md` | All component props | **Required** |
| `bound-value-guide.md` | Data binding patterns | Recommended |
| `styling-guide.md` | Tailwind/theme rules | Recommended |
| `layout-examples.md` | Full JSON examples | Optional (helps quality) |

## Usage Tips

1. **Be specific** - "Create a login form with email and password fields" works better than "make a form"
2. **Mention layout** - "Create a 3-column grid of feature cards" helps the AI understand structure
3. **Data binding** - Mention if you need dynamic data: "Show user name from $.user.name"
4. **Mention responsive needs** - "Make it mobile-friendly with cards stacking on phones" helps enforce layout behavior
5. **Copy the JSON** - The output can be pasted directly into FlowLike's page builder

## Example Prompts

- "Create a pricing page with 3 tier cards (Free, Pro, Enterprise)"
- "Build a user profile card with avatar, name, email, and edit button"
- "Make a dashboard with 4 stat cards and a line chart below"
- "Create a settings form with toggles for notifications, dark mode, and email preferences"
- "Build a product listing grid that binds to $.products data"

This JSON can be imported directly into FlowLike's A2UI page builder.
