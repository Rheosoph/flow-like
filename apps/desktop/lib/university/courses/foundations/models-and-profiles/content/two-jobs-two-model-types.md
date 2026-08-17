Two tickets, one flow. Ticket one needs the flow to *find* the right paragraph in your refund policy. Ticket two needs it to *write* a friendly reply. Same toolkit, but these are different jobs — and in Flow-Like, different jobs mean different model types.

## 1 · The map

@GenAiOverview

The overview picture: on the left, three input tiles — a chat bubble, a document, an envelope — feed into a central model node. From there, connections fan out through a database, a search lens, and a toolbox, converging into two results on the right: a card of generated answer text and a structured table. The lesson hides in the middle of that picture. Between "documents go in" and "answers come out" sits *search* — and search runs on a different engine than writing does.

## 2 · Chat writes, embeddings measure

A **chat model** produces text. Give it history and instructions; get prose, code, or a decision back. In the catalog's PRODUCES filter, that's **Chat** — the Chat & reasoning tab you explored last lesson.

An **embedding model** produces vectors — lists of numbers that place a text's *meaning* in space, so "money back for a broken item" lands near "refund for damaged goods" even though the two share barely a word. You never read an embedding; you compare it against others to find the closest matches. In the catalog that's **Embedding**, a category with its own tab.

The third PRODUCES option, **Speech**, covers voice. Whisper Large v3 sits in the Speech-to-text section — "accurate multilingual speech recognition for meetings and media", says its card — turning audio into text your flows can work with.

Now the rule that saves you a confused afternoon: **a text-generation model is not automatically an embedding, speech, image, or video model.** Capabilities are per-model. If your flow searches, drafts, *and* transcribes, your profile needs a model for each job — one heroic chat model won't secretly do all three.

## 3 · Where each lands in a flow

Lightly here, because the Data course owns retrieval in depth:

- Chat side: **Invoke Model** sends the prepared history to a configured model and returns the response.
- Embedding side: **Load Embedding Model**, then **Embed Document** for the texts you index and **Embed Query** for the question you search with.

Carry one embedding rule out of this course: index and query with the **same embedding model and configuration**. Vectors from different models live in different spaces — comparing them is numerology, not search. Swap the model and you normally rebuild the vector index.

For the support scenario, the shape is set: an embedding model finds the policy passage, a chat model drafts the reply around it, and — when voicemails enter the picture — a speech-to-text model feeds them in. Three jobs, three types, one profile.

Recap:

- Chat produces text; Embedding produces vectors for meaning-based search; Speech covers audio.
- Capabilities are per-model — cover each of a flow's jobs with a model of the right type.
- Index and query with the same embedding model; swapping it means rebuilding the index.
