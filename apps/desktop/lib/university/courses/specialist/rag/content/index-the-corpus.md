Retrieval-augmented generation supplies selected source passages to a model before it answers. Build the evidence index separately from the flow that handles questions.

@PracticeFiles

The fixture contains three short chunks: a refund rule, a support rule and a restricted pricing addendum. They are already chunked so this lab can focus on identity, retrieval and checking evidence. Longer documents need extraction and chunking that preserve sections, tables and provenance; Document Processing covers extraction.

## Index one row, then the others

1. Create a local scratch flow with Simple Event. Add **Open Database**, Table Name `practice_knowledge`.
2. Add **Load Embedding Model** and choose your configured text embedding Model Bit. Pass its Model output to **Embed Document**.
3. Set Embed Document's Query String to the first fixture row's `text`.
4. Add **Set Field** from Structs/Fields. Set its Struct input to the first complete row from `chunks.json`, Field to `vector`, and connect the embedding Vector to Value. Execute it after embedding completes.
5. Pass the updated Struct to **Upsert**, using the Database reference and ID Column `chunk_id`. Follow Success with **Flush Database**; inspect the Error branches.
6. Repeat for the other two rows, using the same embedding model/configuration. Inspect Data Studio: expect three rows with text, provenance, access metadata and equal vector dimensions.
7. Run one row again. Expect three rows, not four.

Connect execution in this order: **Simple Event → Open Database → Load Embedding Model → Embed Document → Set Field → Upsert → Flush Database**. Follow the database nodes' Success outputs; their Error outputs must not continue indexing.

Do not build a large approximate vector index for this three-row exercise. The stored vectors are enough to practice retrieval. A new embedding configuration needs a consistent re-index and matching query embedding; do not mix incompatible vector spaces in one search.
