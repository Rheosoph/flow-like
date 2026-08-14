> **Predict first:** a customer attaches a screenshot of a broken invoice. Your support flow should read it and draft a reply. Nine models sit in the catalog — which ones can even *see* the image?

You don't need a benchmark blog to answer that. Every model card in Flow-Like prints its answer in a single line. This lesson teaches you to read it.

## 1 · Open Explore Models

@ModelCatalog

Click **Explore Models** in the left sidebar. The Models page reports "9 available · 2 in your profile", with a search bar ("Search models, providers, capabilities...") and category tabs: Your profile, Chat & reasoning (6), Speech-to-text (1), Text-to-speech (1), Embeddings (1). Below the search sit two filter rows — ACCEPTS: Text, Image, Audio and PRODUCES: Chat, Embedding, Speech. The exact models you see depend on the current catalog and your profile, so your screen may differ from the screenshot.

## 2 · The card answers three questions

Look at the Gemini 2.5 Flash card: `Text + Image → Text · 1.0M ctx · Hosted`. One line, three questions answered.

**What goes in, what comes out?** The arrow is the capability. `Text + Image → Text` accepts both text and images — it can read that invoice screenshot. Claude Sonnet 4 reads `Text → Text`: brilliant with words, blind to pictures. Whisper Large v3, down in the Speech-to-text section, takes audio in — meetings and voicemails, not screenshots.

**How much fits?** `ctx` is the context window — roughly, how much conversation, document, and instruction the model can consider at once. The spread on this one screen is wide: Gemma 4B Local holds 33K, DeepSeek R1 and Mistral Medium 131K, Claude Sonnet 4 200K, and Gemini 2.5 Flash and GPT-4.1 Mini a roomy 1.0M. A long support thread with pasted policy pages eats context fast.

**Where does it run?** `Hosted` means a provider's infrastructure runs the model — which can require a configured provider or account. `On-device` (Gemma 4B Local is the only one here) means your own machine does the work. That difference matters so much it gets the entire next lesson.

Each card also carries a one-sentence character sketch — DeepSeek R1: "Deliberate reasoning for mathematics, analysis, and complex planning." Useful for shortlisting. But the capability line is the part that can *veto* a choice: no amount of reasoning talent turns `Text → Text` into an image reader.

## 3 · From catalog to toolkit

The IN YOUR PROFILE strip at the top shows Gemini 2.5 Flash and Claude Sonnet 4 with the note "Available to every flow in this workspace". That's the contract: a card marked **In profile** is in your active toolkit, and a card with an **Add to profile** button is one click away from joining it. Once a model is assigned, compatible nodes in Studio — and FlowPilot's model picker — can select it.

(An **Add custom model** button sits in the top-right corner for models the catalog doesn't list. File it under "later".)

So the screenshot answers the prediction: of the nine models, exactly one card lists Image on its accepts side. If reading screenshots is a requirement, that requirement just chose your model for you.

Recap:

- The capability line answers three questions: what goes in and out (the arrow), how much fits (ctx), and where it runs (Hosted / On-device).
- A missing capability is a veto, not a handicap — `Text → Text` will never read a screenshot.
- **Add to profile** is what makes a catalog model usable by your flows.
