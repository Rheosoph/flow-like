Train a classifier, check held-out rows, then reload it for inference. Prerequisites: create native tables and run a Flow. The exercise uses synthetic numeric vectors and needs no language-model provider.

| Target | Task |
| --- | --- |
| Unordered labels | Classification |
| Ordered levels, such as low/medium/high/urgent | Ordinal prediction |
| A continuous quantity | Regression |
| Groups with no target | Clustering |
| Departure from known normal examples | Novelty detection |

The core predicts a package category from `[width, height]`. The deliberately separable examples test pipeline wiring. Their scores say nothing about real parcels.

Download the training, held-out and live rows:

@MLTraining
@MLHoldout
@MLLive

Record the target, feature order and baseline. Both training classes are equally frequent, so always predicting one class gives a 50% training baseline. Keep the held-out labels out of fitting and tuning. Optional OCR/vision classes use separate pretrained models.
