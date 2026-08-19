The tuned category model just passed its held-out test. On Monday it has to label real tickets, inside the real triage flow, without you watching. Between "winner on a board" and "working in production" sits one question most first deployments get wrong.

> **Predict first:** What do you save — one model file, or several? Think about what lesson 2 fitted besides the model.

## 1 · Save the contract, not just the model

**Save Model** persists a trained model through the storage path abstraction; **Load Model** brings it back. (A binary variant pair — Save Model (Binary) and Load Model (Binary) — exists for raw byte handling.)

Here's the catch: each Save Model call accepts **one** model handle. And your classifier is not one model — it's a *contract*. The predictor, the fitted Feature Scaler, the fitted TF-IDF vocabulary: each is its own fitted artifact, each gets its own Save Model call, and the set is versioned **together**. A model card next to the artifacts — training data version, feature schema, target definition, metrics, known limitations — turns "some files" into something a teammate can trust and reproduce.

Skip the scaler and Monday's flow will helpfully refit a fresh one on Monday's tickets — different offsets, different scales, and every prediction quietly shifted. The model didn't drift; the ruler did.

One privacy note: a K-Nearest-Neighbours model embeds a verbatim copy of its training rows, and the TF-IDF vocabulary is verbatim training text. If ticket text is sensitive, the model file is exactly as sensitive — give the artifact the same access controls as the source table.

## 2 · Predict, two ways

The **Predict** node serves the model, and its Data Source pin picks one of two shapes:

**Database mode** is the bulk path: point it at a table, name the Input Col holding the feature vectors and the Output Col to create, and it writes the predicted class for every row, in batches. What it writes is the prediction — no confidence figure appears in the table.

**Vector mode** is the single-row path: feed one feature vector, get back a prediction struct. This struct is the *only* place a `confidence` value exists — and only for models that carry a probability model at all. Logistic Regression reports one; Decision Tree, Random Forest, AdaBoost, and both Naive Bayes variants report none.

That asymmetry decides your architecture. Want to bulk-label the archive? Database mode. Want the live flow to route uncertain tickets to a human? You need Vector mode per ticket, a confidence-capable model, and a threshold branch.

Either way, inference must reproduce training exactly: same feature order, same types, same transforms. Load the *saved* fitted scaler and run **Apply Transform** on the incoming ticket's features *before* Predict — the same replay rule lesson 2 taught for the test split, now applied forever.

## 3 · Into the triage flow

The last mile is ordinary flow-building, which you already know from the Flows and Events courses. The support app below shows Data Studio's overview — one ontology, six object types, two actions in the "Customer Operations" semantic layer — and, bottom-left in the sidebar, a Quick Actions section with a **"Triage selected request"** button.

@DataStudioOverview

That button is the natural front door: its flow prepares the selected ticket's features, replays the fitted transforms, calls Predict, writes category and priority back, and routes anything low-confidence to a human queue. The ML nodes drop into the middle of an ordinary event-driven flow — nothing about serving a model changes how flows run.

**Recap:**

- Ship the contract: predictor + every fitted transformer, saved separately via Save Model, versioned together with a model card.
- Database mode bulk-writes predictions; confidence lives only on Vector mode's struct, and only for probability-carrying models.
- Inference replays the saved fitted transforms with Apply Transform — never refit on incoming data.
