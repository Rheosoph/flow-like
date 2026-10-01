Save the classifier and fitted scaler separately. Each **Save Model** accepts one model handle. Name the pair for one release, such as `parcel-v1-classifier` and `parcel-v1-scaler`, and record training-data version, `[width, height]` order and holdout result.

Start a fresh run. **Load Model** for each artifact, replay the scaler on `ml_live` with **Apply Transform**, then predict from the transformed vectors. Never fit a new scaler on live data.

Expected labels: `small` for `live-1`, `large` for `live-2`. Compare loaded-artifact predictions with the original handles. A difference fails the deployment check.

**Predict** Database mode writes predictions to a table. Vector mode returns a prediction structure and can expose confidence when the chosen model provides probabilities. Database mode does not create a confidence column. A numeric probability also needs validation before being treated as calibrated confidence.

The numeric scaler replays fitted statistics. The TF-IDF corpus limitation still applies; a saved vocabulary is not a complete stable text-feature pipeline. Protect saved artifacts that contain source data, including KNN training rows, according to that data's sensitivity.

Completion: keep artifact names, feature order, two live inputs and both observed labels.
