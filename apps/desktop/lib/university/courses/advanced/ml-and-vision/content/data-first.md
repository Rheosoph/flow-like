Import the JSON assets into three native tables: `ml_train`, `ml_holdout`, `ml_live`. Each row has a unique string `id` and a two-number array `features`. Training and holdout also have `label`. Use the Data course's table-import procedure and confirm arrays are numeric lists, not quoted strings.

1. Add **Fit Feature Scaler**, Data Source `Database`, Method `Standard`, with the training table and Train Col `features`.
2. Connect its Model to **Apply Transform**. Transform the training vectors into a new column, `scaled`.
3. Apply the same fitted scaler to holdout and live, retaining their original vectors.
4. Inspect one row from each: `scaled` must contain two finite numbers in the same order.

The supplied split makes the exercise reproducible. With your own data, split before fitting; **Stratified Split** can preserve class proportions. Keep source and destination tables separate.

**Text-feature limitation:** TF-IDF Apply Transform reuses its vocabulary but recomputes document frequencies from the corpus in each call. Separate calls can therefore produce different feature scales. Saving the vocabulary does not establish identical train/live vectors.

This lab uses a numeric Feature Scaler, whose fitted offsets and scales replay at inference. Keep TF-IDF experiments separate until you demonstrate a supported text-feature contract for the deployment. See [ML preparation](https://docs.flow-like.com/topics/datascience/ml/).
