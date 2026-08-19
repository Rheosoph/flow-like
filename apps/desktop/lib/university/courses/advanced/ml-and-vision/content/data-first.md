You've picked your tasks. Now the raw material: three thousand archived tickets and a customers table. Neither is edible yet — trainers eat numeric feature vectors and a target column, not markdown files and plan names. And there's a step you must take *before* any preparation touches the data, or every score you compute later is quietly inflated.

> **Predict first:** Which comes first — scaling the columns, or splitting the table into train and test? One order is fine. The other invalidates your evaluation.

## 1 · Where the raw material lives

The support app keeps its files on the Storage page. In the screenshot you can see two folders — `archived-tickets` and `customer-briefs` — alongside `brand-voice.md`, `refund-policy.md`, and `support-playbook.pdf`, with buttons to create folders and upload files or whole folders.

@AppStorage

Structured data lives in Data Studio tables. Here's the app's `customers` table: columns for `customer_id`, `name`, `plan`, `status`, `open_tickets`, and `last_contact`, with rows like CUS-1042 (Avery Morgan, Enterprise, Active, 1 open ticket).

@AppDatabases

The Data course owns how tables, SQL, and storage work. What matters for this course: the ML trainers and tuners read their feature column and target column *from a database table*, so "prepare the data" concretely means "get the right numbers into the right columns of a table."

## 2 · Turn words and numbers into vectors

A ticket is mostly text, and a model can't multiply text. Two preprocessing nodes do the conversion:

**Fit TF-IDF Vectorizer** learns a vocabulary from a text column and turns each ticket into a weighted word-count vector — the classic way to make "my invoice is wrong" and "billing error on invoice" land near each other numerically.

**Fit Feature Scaler** puts numeric columns on comparable scales. Without it, `open_tickets` (0–2) whispers while a milliseconds-since-`last_contact` column shouts, and any distance- or gradient-based model hears only the loud one.

Both are *Fit* nodes, and the name is the point: each one **learns** from the data it sees and produces a fitted model of its own. To use a fitted transform on another table — the test split, tomorrow's tickets — you replay it with **Apply Transform**, passing the *same fitted model*. Fitting a second scaler elsewhere produces different statistics, and suddenly train and test aren't measured with the same ruler.

## 3 · Split before anything learns

Now the order question. **Split Dataset** sends rows to a training table and a test table by ratio. The rule: split *first*, then fit the scaler and vectorizer on the **training split only**, then Apply Transform everywhere else.

Why so strict? A scaler fitted on all 3,000 tickets has already peeked at the test rows — their means and spreads are baked into its offsets. Your held-out evaluation is no longer held out; it's been consulted. The leak is silent: nothing errors, the scores just come out flattering.

One refinement for your category target: the archive is lopsided — roughly 70% billing, 20% bug, 10% how-to. A plain random split can leave the test table starved of how-to tickets. **Stratified Split** splits *within each class*, so both tables keep the 70/20/10 shape, and its Seed pin makes the split reproducible.

**Watch out:** TF-IDF is the one exception to clean replay. Apply Transform reuses the fitted vocabulary but recomputes document frequencies from whatever corpus it receives, so vectors are only comparable within a single Apply Transform run. Transform tables together when the numbers must share a scale, and remember the fitted vocabulary is verbatim training text riding inside the saved model.

**Recap:**

- Trainers read features and targets from a table — preparation means filling that table with honest numbers.
- Fit TF-IDF Vectorizer and Fit Feature Scaler *learn* from data; replay them elsewhere with Apply Transform and the same fitted model.
- Split first — Stratified Split for lopsided classes — then fit preprocessing on the training split only.
