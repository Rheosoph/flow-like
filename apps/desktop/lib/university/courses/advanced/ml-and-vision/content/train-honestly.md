Train on `ml_train` and evaluate on `ml_holdout`.

1. Add **Logistic Regression**, Data Source `Database`, with training vector column `scaled` and target `label`. Leave Mode at `Auto`.
2. Connect Model to **Predict**. Select Database mode, the holdout table, input `scaled` and a new prediction column. Use the unique `id` when the node requests row identity.
3. Compare predictions with labels row by row. Pass both column names and the table to **Confusion Matrix**.

Expected fixture result: both small rows and both large rows are classified correctly. If not, inspect feature order, transformed columns and table handles before tuning. Record the observed matrix, including failed cases. Do not change held-out labels to improve a score.

For real data, compare a baseline and inspect costly mistakes. A high overall score can hide one neglected class. Tune on training/validation data and reserve an untouched test set. **Auto Classifier** and **Grid Search** provide further model selection; see [automatic training](https://docs.flow-like.com/topics/datascience/ml-auto-training/).

Ordered targets need **Auto Ordinal**, **Ordinal Grid Search** and **Ordinal Metrics**. Some metrics are errors where lower is better; read **Higher Is Better** when comparing scores. Preserve level order rather than treating levels as interchangeable names.
