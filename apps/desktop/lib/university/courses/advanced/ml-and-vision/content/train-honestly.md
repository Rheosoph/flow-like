Wednesday standup: a teammate has trained a Decision Tree on the ticket archive and announces **99% accuracy**. Ship it Friday? The number is real — and it means almost nothing, because it was measured on the rows the tree trained on. A tree deep enough simply memorizes its training set.

> **Predict first:** What single number do you ask for instead — and which rows must it come from?

## 1 · Select broadly, tune precisely, test once

The model-selection diagram below shows the whole discipline in one line: **"Select broadly. Tune precisely. Test once."** Panel 01, *Auto*, runs candidate families — tree, linear, kernel — through repeated validation folds and produces a leaderboard (tree 0.91, linear 0.87, kernel 0.84). Panel 02, *Grid Search*, takes the winning family and sweeps parameter chips like depth and tree count, refitting on all cross-validation data. Only then does the held-out test — on the dashed *unseen data* line that bypasses everything, marked "outside CV" — verify the result once, and the model is saved as a versioned artifact.

@MLAutoTraining

In the catalog, that's the **Auto Classifier** node: it cross-validates several classifier families on the same folds, ranks them by accuracy or macro-F1, retrains the winner, and reports a `Best Model Type` you can feed straight into **Grid Search** for hyperparameter tuning. The footer of the diagram is your budget line: cost ≈ candidates × folds + one refit. Every value you add to a grid multiplies into that product.

For your **priority** target, though, Auto Classifier is the wrong tuner — and this is where lesson 1 collects its rent. It ranks by accuracy or macro-F1, and both are distance-blind: predicting *low* on an *urgent* ticket scores exactly like predicting *high*. The leaderboard will look completely plausible and still crown the wrong model. Ordered targets get their own pair: **Auto Ordinal** and **Ordinal Grid Search**, which keep the level order end to end and rank by distance-aware metrics.

One sharp edge in the ordinal metrics: two of them — Mean Rank Error and Macro Rank Error — are *error* metrics, where **lower** is better. Rank them the wrong way round and you crown the worst candidate in the sweep. That's why Ordinal Grid Search publishes a dedicated `Higher Is Better` output pin next to `Best Score`: branch on it instead of assuming bigger wins.

## 2 · Read the metric that hurts

Once a winner exists, evaluate it on the untouched test split — the one Stratified Split carved off in lesson 2, which no tuner ever saw.

For **category**, don't stop at the **Accuracy** node. With 70% billing tickets, a model that answers "billing" every time scores 70% while being useless. Read the **Confusion Matrix**: it shows *which* classes leak into which, and whether how-to — your smallest class — is being found at all.

For **priority**, use **Ordinal Metrics**. Its headline is quadratic weighted kappa, which charges a two-level miss four times as much as a one-level miss — exactly the cost structure your support team feels when an urgent ticket lands in the low pile.

And the teammate's 99%? Ask for the score on the held-out test split. If training accuracy is 99% and test accuracy is 78%, the gap *is* the diagnosis: the tree memorized. The first lever is capacity — lower Max Depth — not more training.

## 3 · Turn one knob family at a time

When you do tune by hand, tune deliberately. The configuration map groups every knob into five families — data shape (scale, encode, order), objective (link, loss, margin), capacity (kernel, depth, hidden layers), control (regularization, optimizer), and diagnostics (convergence, crossings, spread) — all feeding one model fit, then validation. Its footer is the rule worth framing: **"Turn one family of settings at a time."**

@MLConfigurationMap

Change capacity and control together and you won't know which change moved the score — you've traded a measurement for a coin flip.

**Recap:**

- Auto node picks the family, Grid Search tunes it, and the held-out test verifies **once** — it never joins the tuning loop.
- Ordered targets need the ordinal tuners and Ordinal Metrics; accuracy scores a four-level miss like a one-level one.
- A training-set score isn't evidence. The train-versus-test gap is your overfitting gauge, and capacity is the first lever.
