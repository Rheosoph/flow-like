Your support queue tripled this quarter. Every ticket needs three fields the team currently fills by hand: a **category** (billing, bug, or how-to), a **priority** (low, medium, high, urgent), and an estimate of **hours to resolve**. Someone in standup says "let's just train an AI on the archive." One wish — but it hides three different machine-learning problems, and picking the wrong one produces a model that looks fine and triages badly.

> **Predict first:** Can one model type handle all three fields? If not, what actually decides which type each field needs?

## 1 · Frame — the target decides, not the algorithm

The answer isn't in the algorithm catalog. It's in the column you want filled. Look at the shape of the target and the task follows.

The decision guide below starts from exactly one question — "What does the target look like?" — and routes five target shapes to five tasks: unordered labels go to classification, ordered levels to ordinal, a continuous number to regression, no target at all to clustering, and only-normal examples to novelty detection.

@MLTaskDecisionGuide

Run your three ticket fields through it:

**Category is classification.** Billing, bug, how-to — names with no inherent order. The deciding test: reorder the labels and nothing about the problem changes. That's the classification branch, and Flow-Like's classifier trainers (Decision Tree, Random Forest, Logistic Regression, and friends) live there.

**Priority is ordinal — not classification.** Low < medium < high < urgent is an *order*, and the order carries information. A classifier throws it away: predicting *low* when the truth is *urgent* costs it exactly as much as predicting *high*. An ordinal model knows a three-level miss is worse than a one-level miss. It's also not regression — a regressor invents distances the levels don't carry, as if *urgent* were exactly twice *high*.

**Hours to resolve is regression.** A continuous number where differences mean something: the gap between 2 and 4 hours means the same as between 20 and 22.

Two more branches will earn their keep later. When you have no target — say you suspect the archive hides ticket *types* nobody has named yet — that's **clustering**: you want groups, but nothing supplies the correct groups. And when you only have examples of normal traffic and want the weird stuff flagged, that's **novelty detection** with a One-Class SVM: it answers inlier or outlier, not which class.

## 2 · The pipeline you'll build

Whatever the task, the shape of the work is the same, and it's the shape this course walks lesson by lesson. The banner diagram shows it as one glowing flow: a data table feeds a split step, then a cleaning-and-preparation step, then the central model node; the model's output flows into an evaluation card with charts, which branches into a saved-model database and a prediction step. Above the model, five small cards — a decision boundary, a fitted line, ordered bars, dashed cluster circles, a 3D plane — represent the task families feeding into it.

@MLPipelineOverview

That's the whole promise of ML in Flow-Like: every one of those stages is a node on a board. The split you chose, the scaler you fitted, the metric you read — all inspectable, all re-runnable, none of it buried in a notebook.

## 3 · Where this course sits

Zoom out once. The data-science workflow diagram shows five stages — 01 Load data (files, APIs, databases and lakes), 02 Explore (DataFusion SQL, clean and transform), 03 Analyze, 04 Visualize (charts, tables, dashboards), 05 Deploy (schedules, APIs, chat) — with a feedback loop from Deploy back to Load.

@DataScienceOverview

Stage 03 offers two kinds of intelligence: *machine learning — train and predict* and *GenAI agents — investigate and explain*. This course owns the first one, plus running pretrained vision and text models via ONNX in lessons 5 and 6. Agents, RAG, and LLM prompting belong to the Agentic AI course; loading and querying the data itself is the Data course's turf. You'll borrow both and re-teach neither.

**Recap:**

- The target column's shape picks the task: unordered labels → classification, ordered levels → ordinal, continuous number → regression, no target → clustering, normal-only → novelty detection.
- Priority is the trap: it looks like classification, but the order carries information a classifier discards.
- Training in Flow-Like is a flow — split, prepare, train, evaluate, save, predict — one inspectable node per stage.
