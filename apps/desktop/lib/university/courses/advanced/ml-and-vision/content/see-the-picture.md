Optional: check an ONNX object detector against your own labelled images. Obtain a compatible export and its class list/preprocessing instructions before starting.

| Required result | Node |
| --- | --- |
| One label for a whole image | Image Classification |
| Objects with positions | Object Detection |
| A class for each pixel | Semantic Segmentation |

Choose a harmless household object in the detector's class list. Create one image with it visible and one without it; add expectations to the supplied label sheet. These images are your fixture evidence.

Load the model, inspect **Model Info**, reproduce preprocessing and run both images. Compare class and box position with the visible object. A class absent from the export's training labels is a model-selection problem.

Repeat with the object partly obscured. Record misses or changed confidence. **Batch Image Inference** reuses one session for an image list once the individual path works.

Completion: retain expected and observed results for all three inputs. A published benchmark does not measure these images. More model families are in the [ML reference](https://docs.flow-like.com/topics/datascience/ml/).
