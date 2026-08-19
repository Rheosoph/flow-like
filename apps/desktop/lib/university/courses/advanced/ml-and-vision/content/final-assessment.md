Quarter's end. The support team wants the whole thing live: the Triage Machine. You're the reviewer for the release. Below are the requirements, the build log from your teammates, and what the dry run produced. The challenges ask you to judge each decision — everything you need was covered across the six lessons, and nothing here will re-explain it.

## The requirements

- **R1** — Every new ticket gets a **category**: billing, bug, or how-to. The archive runs roughly 70% billing, 20% bug, 10% how-to.
- **R2** — Every new ticket gets a **priority**: low < medium < high < urgent. Sending an urgent ticket to the low queue is the costliest possible miss.
- **R3** — Scanned or photographed invoices are read automatically, and `invoice_number`, `amount`, and `due_date` become ticket fields.
- **R4** — Photos of damaged hardware are flagged when the product is visible, with its position marked for the human reviewer.
- **R5** — Predictions run inside the triage flow. Tickets the model is unsure about go to a human queue instead of being auto-filed.

## The build log

- **B1** — For the category model, a teammate fitted the Feature Scaler and TF-IDF Vectorizer on all 3,000 archived tickets, then split the table 80/20 and trained on the 80.
- **B2** — Priority was tuned with Auto Classifier ranking by accuracy. The leaderboard looked clean and a winner was retrained.
- **B3** — The inference flow, "to stay adaptive," fits a fresh Feature Scaler on each day's incoming tickets before Predict.
- **B4** — The invoice reader wires the full attachment image straight into Text Recognition. Field extraction hasn't been chosen yet.
- **B5** — The hardware-photo step and the attachment backlog (about 4,000 archived images) are still unassigned.
- **B6** — The ship list says: "one Save Model call — it bundles the classifier with its preprocessing."
- **B7** — A teammate proposes switching category to K-Nearest Neighbours and notes ticket text includes customer account details.

## The dry run

- **D1** — Category test accuracy: **99.4%**. The team is celebrating; the reviewer (you) is not.
- **D2** — Priority leaderboard scores looked plausible, but in shadow mode several *urgent* tickets were filed as *low*.
- **D3** — On Monday's live tickets, category predictions skewed heavily toward billing — far beyond the archive's 70%.
- **D4** — The invoice reader returns strings of gibberish on every attachment.
- **D5** — Uncertain-ticket routing (R5) has no design yet: nobody knows where a per-ticket confidence would come from.

Work through the challenges. Each one hands you a requirement, a build-log entry, or a dry-run symptom — your job is the diagnosis and the fix.
