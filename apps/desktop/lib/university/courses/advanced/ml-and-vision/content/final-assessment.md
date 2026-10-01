Verify the saved numeric model from a fresh run. Optional OCR and vision are not assessed here.

1. Load classifier and scaler without calling any fit node.
2. Predict both live fixture rows. Expect `small`, then `large`.
3. Copy `live-1` with a new ID and identical features. Its prediction must match the original when processed with the other rows.
4. Deliberately swap the width and height inputs where the vector is assembled. Compare that vector with the named source values, then restore the declared order. A two-number schema cannot detect this permutation by itself; the assembly check must establish the mapping.

Keep the held-out matrix, artifact pair, declared feature order and observed predictions. Record failures and repairs. This is manual evidence; the questions do not inspect your saved model.
