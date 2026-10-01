A model assignment is useful only if it matches the operation the workflow needs.

- Text generation produces a response from instructions and context.
- Embedding produces numeric vectors used to compare meaning.
- Speech-to-text turns audio into text; text-to-speech produces audio.

For a meeting assistant, write a three-row plan: transcribe a synthetic recording, retrieve related notes, draft a summary. Name the required model capability for each row and identify a compatible option in your catalogue. One model can cover multiple rows only if its configured capabilities actually do.

Keep an embedding index and its queries on the same model and configuration. Changing the embedding model normally requires rebuilding the index; a new vector with the same length is not automatically comparable.

**Check:** the plan covers all three jobs and records the embedding model/configuration. Continue with **Build and Evaluate RAG** to implement retrieval.
