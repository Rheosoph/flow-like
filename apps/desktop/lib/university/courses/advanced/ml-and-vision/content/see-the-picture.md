Not every attachment is an invoice. The support inbox also collects screenshots of error dialogs and photos of dented hardware fresh off the courier. "The product arrived damaged — see photo" needs a different answer than "here's my invoice": not *what does the text say*, but *what's in this picture, and where?*

> **Predict first:** "Is there a product in this photo?" and "where in the frame is it?" — one node, or two different capabilities?

## 1 · Three questions, three nodes

Vision nodes divide by the question they answer, and picking one is picking a question:

**Image Classification** answers *"which one is this?"* — one label for the whole image, from exports like MobileNet, ResNet, SqueezeNet, or EfficientNet. Perfect for routing: is this attachment a screenshot, a photo, or a scanned document?

**Object Detection** answers *"what's here, and where?"* — a list of detected objects with bounding boxes, from exports like YOLO variants and SSD-MobileNet. This is the damaged-hardware question: is the product visible, and where in the frame, so a human can look exactly there.

**Semantic Segmentation** answers *"which pixels belong to which class?"* — a per-pixel class map, from exports like DeepLabV3 and FCN. It's the heavyweight: reach for it when boundaries matter (how much of the surface is affected), not when a box would do.

Same discipline as lesson 5 for all three: these are pretrained ONNX exports. Load ONNX, check the expected tensors with Model Info, reproduce the preprocessing, and remember a model trained on general photo datasets has never seen *your* products — its confident answer is still a hypothesis until you've checked it against your own attachments.

## 2 · The supporting cast

The ONNX catalog goes well past those three, all at the same "load an export and run it" level:

- **Face Detection**, **Face Embedding**, and **Compare Faces** for finding and matching faces.
- **Pose Estimation** with **Extract Keypoint** for body keypoints.
- **Depth Estimation** for per-pixel depth, with helpers to colorize it or lift it to a point cloud.
- **Feature Extraction** and **Feature Similarity** — turn any image into an embedding vector and compare embeddings. That pair is your near-duplicate hunter: "this exact error screenshot arrived forty times this week" is one similarity threshold away.
- **Batch Image Inference** — run one loaded model across a whole *list* of images in batches. This is how the `archived-tickets` folder's attachment backlog gets processed without wiring a manual loop of single inferences.

Where a business case needs faces, be deliberate: biometric data has legal weight in most jurisdictions the moment you process it. Governance guardrails live in the App Governance course.

## 3 · Trust, but verify — with labels you own

Vision models don't get the luxury of a training leaderboard — you didn't train them — but lesson 3's honesty rule still applies, just relocated: **build your own held-out reality check.** Take a sample of real attachments, label them by hand (screenshot / photo / document; product visible / not), run the model, and compare. A published benchmark score describes the model's home dataset, not your inbox. Twenty minutes of hand-labeling buys you the only number that predicts Monday.

If the check passes, the routing flow assembles from parts you already own: Image Classification sorts attachment type, invoices take lesson 5's OCR relay, photos take Object Detection, and results land back in the triage flow's ticket record.

**Recap:**

- Pick the node by the question: which one → Image Classification; what and where → Object Detection; which pixels → Semantic Segmentation.
- Feature Extraction + Feature Similarity find near-duplicates; Batch Image Inference processes image lists at scale.
- Benchmark scores aren't your scores — verify every pretrained model against a hand-labeled sample of your own data.
