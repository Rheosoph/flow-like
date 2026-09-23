// onnx — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ml {
    // === AI/ML ===

    /**
     * Extract class_idx and label from predictions.
     * @node ai_ml_pred_class_or_label @receiver prediction @alias aiMlPredClassOrLabel
     * @param prediction — Single ClassPrediction (receiver: `this` in `x.classOrLabel(...)`)
     * @returns classIdx — Selected prediction class index
     * @returns label — Selected prediction label (empty if not provided)
     */
    function classOrLabel(this: ClassPrediction, { prediction: Struct }): { classIdx: int, label: string };

    // === AI/ML/Teachable Machine ===

    /**
     * Extract score from predictions.
     * @node ai_ml_pred_score @receiver prediction @alias aiMlPredScore
     * @param prediction — Single ClassPrediction (receiver: `this` in `x.score(...)`)
     * @returns score — Selected prediction score
     */
    function score(this: ClassPrediction, { prediction: Struct }): float;

    namespace teachableMachine {
        // === AI/ML ===

        /**
         * Image classification using Teachable Machine models.
         * @node ai_ml_teachable_machine @alias aiMlTeachableMachine
         * @param model — Path to *.tflite model
         * @param imageIn — Image Object
         * @param labels — Optional labels.txt
         * @param inputWidth (optional) — Model input width
         * @param inputHeight (optional) — Model input height
         * @returns predictions — Class Predictions
         * @impure has side effects / drives control flow
         */
        function classify({ model: Struct, imageIn: Struct, labels: Struct, inputWidth?: int, inputHeight?: int }): Struct[];
    }
}

declare namespace onnx {
    // === AI/ML/ONNX ===

    /**
     * Extract a specific keypoint from a pose by index or name
     * @node extract_keypoint @alias extractKeypoint
     * @param pose — Pose detection to extract keypoint from
     * @param keypointIdx (optional) — Keypoint index (0-based)
     * @returns x — Keypoint X coordinate
     * @returns y — Keypoint Y coordinate
     * @returns confidence — Keypoint confidence score
     * @returns name — Keypoint name (if available)
     * @returns found — Whether the keypoint was found
     */
    function extractKeypoint({ pose: Struct, keypointIdx?: int }): { x: float, y: float, confidence: float, name: string, found: bool };

    /**
     * Extract feature vectors from images using ONNX models
     * @node feature_extraction @alias featureExtraction
     * @param model — ONNX Model Session
     * @param imageIn — Image Object
     * @param normalize (optional) — Normalize output to unit length
     * @returns features — Extracted feature vector
     * @returns dimensions — Feature vector dimensionality
     * @impure has side effects / drives control flow
     */
    function featureExtraction({ model: Struct, imageIn: Struct, normalize?: bool }): { features: Struct, dimensions: int };

    /**
     * Compare two feature vectors using cosine similarity or L2 distance
     * @node feature_similarity @alias featureSimilarity
     * @param featuresA — First feature vector
     * @param featuresB — Second feature vector
     * @returns cosineSimilarity — Cosine similarity (-1 to 1, higher is more similar)
     * @returns l2Distance — Euclidean distance (lower is more similar)
     */
    function featureSimilarity({ featuresA: Struct, featuresB: Struct }): { cosineSimilarity: float, l2Distance: float };

    /**
     * Image Classification with ONNX-Models. Download models from: MobileNetV2 (https://github.com/onnx/models/tree/main/validated/vision/classification/mobilenet), SqueezeNet (https://github.com/onnx/models/tree/main/validated/vision/classification/squeezenet), ResNet (https://github.com/onnx/models/tree/main/validated/vision/classification/resnet), EfficientNet (https://github.com/onnx/models/tree/main/validated/vision/classification/efficientnet-lite4)
     * @node image_classification @alias imageClassification
     * @param model — ONNX Model Session
     * @param imageIn — Image Object
     * @param mean (optional) — Image Mean for Normalization (per channel)
     * @param std (optional) — Image Standard Deviation for Normalization (per channel)
     * @param cropPct (optional) — Center Crop Percentage
     * @param softmax (optional) — Scale Outputs with Softmax
     * @returns predictions — Class Predictions
     * @impure has side effects / drives control flow
     */
    function imageClassification({ model: Struct, imageIn: Struct, mean?: float[], std?: float[], cropPct?: float, softmax?: bool }): Struct[];

    /**
     * Get information about a loaded ONNX session
     * @node onnx_session_info @receiver model @alias onnxSessionInfo
     * @param model — ONNX Model Session (receiver: `this` in `x.info(...)`)
     * @returns inputs — List of model inputs
     * @returns outputs — List of model outputs
     * @returns inputNames — Comma-separated input names
     * @returns outputNames — Comma-separated output names
     */
    function info(this: NodeOnnxSession, { model: Struct }): { inputs: Struct[], outputs: Struct[], inputNames: string, outputNames: string };

    /**
     * Load ONNX Model from Path
     * @node load_onnx @alias loadOnnx
     * @param path — Path ONNX File
     * @returns model — ONNX Model Session
     * @returns accelerated — Whether a GPU/NPU execution provider was configured; individual sessions may still fall back to CPU
     * @returns activeProvider — Execution providers configured in priority order, including CPU fallback
     * @impure has side effects / drives control flow
     */
    function load({ path: Struct }): { model: Struct, accelerated: bool, activeProvider: string };

    /**
     * Get ONNX model metadata (inputs, outputs, shapes)
     * @node onnx_model_info @alias onnxModelInfo
     * @param path — Path to ONNX file
     * @returns metadata — Model metadata
     * @returns inputs — List of model inputs
     * @returns outputs — List of model outputs
     * @impure has side effects / drives control flow
     */
    function modelInfo({ path: Struct }): { metadata: Struct, inputs: Struct[], outputs: Struct[] };

    /**
     * Object Detection in Images with ONNX-Models. Download models from: TinyYOLOv2 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/tiny-yolov2), YOLO (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation), SSD-MobileNet (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1)
     * @node object_detection @alias objectDetection
     * @param model — ONNX Model Session
     * @param imageIn — Image Object
     * @param conf (optional) — Confidence Threshold
     * @param iou (optional) — Intersection Over Union Threshold for NMS
     * @param max (optional) — Maximum Number of Detections
     * @returns bboxes — Bounding Box Predictions
     * @impure has side effects / drives control flow
     */
    function objectDetection({ model: Struct, imageIn: Struct, conf?: float, iou?: float, max?: int }): Struct[];

    /**
     * Detect human poses and keypoints using ONNX models. Download models from: YOLOv8-Pose (https://docs.ultralytics.com/models/yolov8/), MoveNet (https://tfhub.dev/google/movenet/), HRNet (https://github.com/OAID/TengineKit)
     * @node pose_estimation @alias poseEstimation
     * @param model — ONNX Model Session
     * @param imageIn — Image Object
     * @param conf (optional) — Minimum keypoint confidence threshold
     * @param maxPoses (optional) — Maximum number of poses to detect
     * @returns poses — Detected poses with keypoints
     * @impure has side effects / drives control flow
     */
    function poseEstimation({ model: Struct, imageIn: Struct, conf?: float, maxPoses?: int }): Struct[];

    /**
     * Segment images into semantic classes using ONNX models. Download models from: DeepLabV3 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/duc), FCN (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/fcn)
     * @node semantic_segmentation @alias semanticSegmentation
     * @param model — ONNX Model Session
     * @param imageIn — Image Object
     * @param numClasses (optional) — Number of segmentation classes
     * @returns mask — Segmentation mask output
     * @impure has side effects / drives control flow
     */
    function semanticSegmentation({ model: Struct, imageIn: Struct, numClasses?: int }): Struct;

    /**
     * Release ONNX model from cache to free memory
     * @node unload_onnx @receiver model @alias unloadOnnx
     * @param model — ONNX Model Session to unload (receiver: `this` in `x.unload(...)`)
     * @returns success — Whether the model was successfully unloaded
     * @impure has side effects / drives control flow
     */
    function unload(this: NodeOnnxSession, { model: Struct }): bool;

    // === AI/ML/ONNX/Audio ===

    /**
     * Convert audio to mel spectrogram for speech models
     * @node audio_to_mel_spectrogram @alias audioToMelSpectrogram
     * @param audio — Input audio (16kHz mono)
     * @param nMels (optional) — Number of mel bands
     * @param hopLength (optional) — Hop length in samples
     * @param nFft (optional) — FFT window size
     * @returns spectrogram — Mel spectrogram [n_mels, time]
     * @returns frames — Number of time frames
     * @impure has side effects / drives control flow
     */
    function audioToMelSpectrogram({ audio: Struct, nMels?: int, hopLength?: int, nFft?: int }): { spectrogram: any, frames: int };

    /**
     * Load audio file for processing
     * @node load_audio @alias loadAudio
     * @param path — Path to audio file
     * @returns audio — Loaded audio data
     * @returns sampleRate — Audio sample rate
     * @returns duration — Duration in seconds
     * @impure has side effects / drives control flow
     */
    function loadAudio({ path: Struct }): { audio: Struct, sampleRate: int, duration: float };

    /**
     * Resample audio to target sample rate
     * @node resample_audio @alias resampleAudio
     * @param audio — Input audio
     * @param targetRate (optional) — Target sample rate
     * @param toMono (optional) — Convert to mono
     * @returns audioOut — Resampled audio
     * @impure has side effects / drives control flow
     */
    function resampleAudio({ audio: Struct, targetRate?: int, toMono?: bool }): Struct;

    /**
     * Trim audio to speech segments from VAD
     * @node trim_audio @alias trimAudio
     * @param audio — Input audio
     * @param segments — Speech segments from VAD
     * @param padding (optional) — Padding around segments (seconds)
     * @returns clips — Trimmed audio clips
     * @impure has side effects / drives control flow
     */
    function trimAudio({ audio: Struct, segments: any, padding?: float }): any;

    /**
     * Detect speech segments in audio. Download Silero VAD model from: https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx
     * @node onnx_vad @alias onnxVad
     * @param model — ONNX VAD Model
     * @param audio — Input audio data
     * @param threshold (optional) — Speech probability threshold
     * @param minSpeechMs (optional) — Minimum speech duration (ms)
     * @param minSilenceMs (optional) — Minimum silence duration (ms)
     * @returns result — VAD result
     * @returns segments — Speech segments
     * @impure has side effects / drives control flow
     */
    function vad({ model: Struct, audio: Struct, threshold?: float, minSpeechMs?: int, minSilenceMs?: int }): { result: Struct, segments: any };

    // === AI/ML/ONNX/Batch ===

    /**
     * Run ONNX inference on multiple images in batches
     * @node onnx_batch_image_inference @alias onnxBatchImageInference
     * @param model — ONNX Model Session
     * @param images — List of images to process
     * @param batchSize (optional) — Number of images per batch
     * @param inputSize (optional) — Model input size
     * @param normalize (optional) — Apply ImageNet normalization
     * @returns results — Raw output tensors per image
     * @returns batchResult — Batch processing summary
     * @impure has side effects / drives control flow
     */
    function batchImageInference({ model: Struct, images: any, batchSize?: int, inputSize?: int, normalize?: bool }): { results: any, batchResult: Struct };

    // === AI/ML/ONNX/Face ===

    /**
     * Compare two face embeddings for similarity
     * @node compare_faces @alias compareFaces
     * @param embeddingA — First face embedding
     * @param embeddingB — Second face embedding
     * @param threshold (optional) — Match threshold (cosine similarity)
     * @returns isMatch — Whether faces match
     * @returns similarity — Cosine similarity score
     * @returns distance — Euclidean distance
     * @impure has side effects / drives control flow
     */
    function compareFaces({ embeddingA: Struct, embeddingB: Struct, threshold?: float }): { isMatch: bool, similarity: float, distance: float };

    /**
     * Crop detected faces from image
     * @node crop_faces @alias cropFaces
     * @param image — Source image
     * @param faces — Detected faces
     * @param margin (optional) — Margin around face (fraction)
     * @returns crops — Cropped face images
     * @impure has side effects / drives control flow
     */
    function cropFaces({ image: Struct, faces: any, margin?: float }): any;

    /**
     * Detect faces in images. Download models from: UltraFace (https://github.com/onnx/models/tree/main/validated/vision/body_analysis/ultraface), RetinaFace (https://huggingface.co/arnabdhar/retinaface-onnx), SCRFD (https://huggingface.co/onnx-community/scrfd_10g_bnkps)
     * @node onnx_face_detection @alias onnxFaceDetection
     * @param model — ONNX Face Detection Model
     * @param image — Input Image
     * @param threshold (optional) — Detection confidence threshold
     * @param nmsThreshold (optional) — Non-maximum suppression threshold
     * @param inputSize (optional) — Model input size
     * @returns faces — Detected faces
     * @returns count — Number of detected faces
     * @impure has side effects / drives control flow
     */
    function faceDetection({ model: Struct, image: Struct, threshold?: float, nmsThreshold?: float, inputSize?: int }): { faces: any, count: int };

    /**
     * Extract face embedding for recognition. Download models from: ArcFace (https://huggingface.co/onnx-community/arcface_torch/tree/main), FaceNet (https://huggingface.co/rocca/facenet-onnx)
     * @node onnx_face_embedding @alias onnxFaceEmbedding
     * @param model — ONNX Face Embedding Model
     * @param image — Aligned face image
     * @param inputSize (optional) — Model input size (typically 112 or 160)
     * @returns embedding — Face embedding vector
     * @impure has side effects / drives control flow
     */
    function faceEmbedding({ model: Struct, image: Struct, inputSize?: int }): Struct;

    /**
     * Detect faces and extract embeddings, gender and age using a face_id analyzer
     * @node face_id_analyze @alias faceIdAnalyze
     * @param analyzer — Face analyzer handle
     * @param image — Input Image
     * @param maxFaces (optional) — Maximum number of faces to embed and analyze
     * @returns faces — Analyzed faces
     * @returns count — Number of detected faces
     * @impure has side effects / drives control flow
     */
    function faceIdAnalyze({ analyzer: Struct, image: Struct, maxFaces?: int }): { faces: Struct[], count: int };

    /**
     * Load a face_id analyzer (SCRFD detector + ArcFace embedder + gender/age). Weights are verified and cached when a session identity is first built; equivalent analyzers reuse process-wide sessions.
     * @node face_id_load_analyzer @alias faceIdLoadAnalyzer
     * @param cacheDir — FlowPath used when this analyzer identity needs to build its ONNX sessions. If it is already resident, an alternate cache directory is not populated.
     * @param detectorUrl (optional) — Immutable SCRFD detector weights URL
     * @param detectorSha256 (optional) — Required SHA-256 checksum for the detector weights
     * @param embedderUrl (optional) — Immutable ArcFace recognition weights URL
     * @param embedderSha256 (optional) — Required SHA-256 checksum for the recognition weights
     * @param genderAgeUrl (optional) — Immutable gender & age estimation weights URL
     * @param genderAgeSha256 (optional) — Required SHA-256 checksum for the gender & age weights
     * @param inputSize (optional) — Square detector input size
     * @param scoreThreshold (optional) — Detector confidence threshold
     * @param iouThreshold (optional) — Detector non-maximum-suppression IoU threshold
     * @returns analyzer — Cached face analyzer handle
     * @impure has side effects / drives control flow
     */
    function faceIdLoadAnalyzer({ cacheDir: Struct, detectorUrl?: string, detectorSha256?: string, embedderUrl?: string, embedderSha256?: string, genderAgeUrl?: string, genderAgeSha256?: string, inputSize?: int, scoreThreshold?: float, iouThreshold?: float }): Struct;

    /**
     * Release a cached face analyzer and its three ONNX sessions. Equivalent analyzer handles share the same cache entry and are invalidated together.
     * @node face_id_unload_analyzer @alias faceIdUnloadAnalyzer
     * @param analyzer — Face analyzer handle to unload
     * @returns success — Whether a face analyzer cache entry was removed
     * @impure has side effects / drives control flow
     */
    function faceIdUnloadAnalyzer({ analyzer: Struct }): bool;

    // === AI/ML/ONNX/NLP ===

    /**
     * Extract entities for any labels you name at runtime, with no fixed label set and no retraining. Load a GLiNER ONNX export (e.g. https://huggingface.co/onnx-community/gliner_small-v2.1, gliner_multi-v2.1, gliner_medium_news-v2.1, gliner_multi_pii-v1, NuNER_Zero) plus the tokenizer.json from the same repository. For models with a fixed label set, use the Named Entity Recognition node instead.
     * @node onnx_gliner @alias onnxGliner
     * @param model — ONNX GLiNER Model Session
     * @param tokenizer — HuggingFace tokenizer.json from the same model repository
     * @param text — Input text to analyze for named entities
     * @param labels — Entity types to look for, in plain language (e.g. person, company, medication, invoice number)
     * @param threshold (optional) — Minimum confidence for a span to be reported (0.0-1.0)
     * @param maxWidth (optional) — Longest entity in words. Must match the model's max_width from gliner_config.json (12 for most GLiNER models, 1 for NuNER Zero)
     * @param multiLabel (optional) — Report every label that clears the threshold for a span instead of only the best one
     * @param mergeAdjacent (optional) — Join neighbouring same-label entities separated only by whitespace. Required for token-level models such as NuNER Zero, which score one word at a time
     * @returns result — Full zero-shot result with entities and the labels that were requested
     * @returns entities — Extracted entities as array
     * @returns entityCount — Number of entities found
     * @impure has side effects / drives control flow
     */
    function gliner({ model: Struct, tokenizer: Struct, text: string, labels: string[], threshold?: float, maxWidth?: int, multiLabel?: bool, mergeAdjacent?: bool }): { result: Struct, entities: Struct[], entityCount: int };

    /**
     * Answer a choice, rubric score, or true-probability question about text using Laya multilingual. Supply model files as FlowPaths, or connect a cache directory to download missing files from mizchi/laya-multilingual-onnx (about 681 MB). Loaded models are reused within the execution cache.
     * @node onnx_laya @alias onnxLaya
     * @param weights (optional) — Optional model.onnx FlowPath. If omitted, download the pinned multilingual weights into Cache Directory
     * @param tokenizer (optional) — Optional matching tokenizer/tokenizer.json FlowPath. If omitted, use the pinned multilingual tokenizer
     * @param config (optional) — Optional matching rl_agent_config.json FlowPath with prompt limits and temperatures. If omitted, use the pinned multilingual config
     * @param cacheDir (optional) — FlowPath directory for verified downloads. Required when any model file is omitted. The complete bundle uses about 681 MB of cache space
     * @param text — Text or serialized JSON state to evaluate
     * @param instructions — Question to answer about Text
     * @param questionType (optional) — choice selects a label; score returns an expected rubric level; noul returns P(true)
     * @param criteria (optional) — Unique choice labels or ordered score descriptions (level 0 first). For noul, leave empty or supply false then true descriptions
     * @returns result — Typed answer, calibrated probabilities, confidence, action probability and token count
     * @returns choice — Selected label for choice questions; empty for other types
     * @returns score — Expected zero-based rubric level for score questions; zero for other types
     * @returns noul — Probability the statement holds for noul questions; zero for other types
     * @returns confidence — Normalized entropy confidence, or max(P(false), P(true)) for noul
     * @impure has side effects / drives control flow
     */
    function laya({ weights?: Struct, tokenizer?: Struct, config?: Struct, cacheDir?: Struct, text: string, instructions: string, questionType?: string, criteria?: string[] }): { result: Struct, choice: string, score: float, noul: float, confidence: float };

    /**
     * Extract named entities (persons, organizations, locations, dates, etc.) from text using ONNX models. Supports BERT, RoBERTa, and other transformer-based NER models with automatic tokenization. Download models from: BERT-base-NER (https://huggingface.co/dslim/bert-base-NER), Multilingual NER (https://huggingface.co/Davlan/bert-base-multilingual-cased-ner-hrl), spaCy NER (https://huggingface.co/spacy). Text longer than the model's window is split into overlapping chunks rather than truncated, so entities are found throughout a long document. Download tokenizer.json and config.json from the same model repository — config.json carries the id2label mapping that names the entity types and the sequence length the model accepts.
     * @node onnx_ner @alias onnxNer
     * @param model — ONNX NER Model Session
     * @param tokenizer — HuggingFace tokenizer.json file for BERT/RoBERTa tokenization. Download from the same model repository.
     * @param config — HuggingFace config.json of the model. Supplies the id2label mapping that decides which class index means which entity type, and max_position_embeddings, which sets how many tokens fit in one window. Left empty, the node looks for config.json next to the tokenizer. Strongly recommended: label orderings differ between models of the same size, and a wrong one mislabels every entity.
     * @param text — Input text to analyze for named entities
     * @param labels — Entity label names in model output order (e.g. ['O', 'B-PER', 'I-PER', 'B-ORG', ...]). Overrides the Config pin. If both are empty, the node falls back to the CoNLL-2003 ordering of dslim/bert-base-NER.
     * @param scheme (optional) — Tagging scheme: BIO, BIOES, IOB, or BILOU
     * @param threshold (optional) — Minimum confidence threshold for entity extraction (0.0-1.0)
     * @returns result — Full NER result with entities and token predictions
     * @returns entities — Extracted named entities as array
     * @returns entityCount — Number of entities found
     * @impure has side effects / drives control flow
     */
    function ner({ model: Struct, tokenizer: Struct, config: Struct, text: string, labels: string[], scheme?: Struct, threshold?: float }): { result: Struct, entities: Struct[], entityCount: int };

    // === AI/ML/ONNX/OCR ===

    /**
     * Crop detected text regions from image for recognition
     * @node crop_text_regions @alias cropTextRegions
     * @param image — Source image
     * @param regions — Detected text regions
     * @param padding (optional) — Padding around regions (pixels)
     * @returns crops — Cropped region images
     * @impure has side effects / drives control flow
     */
    function cropTextRegions({ image: Struct, regions: any, padding?: int }): any;

    /**
     * Detect text regions in images. Download models from: CRAFT (https://huggingface.co/quocanh34/craft_text_detection_onnx), DBNet (https://huggingface.co/Xenova/dbnet_resnet50_onnx), EAST (https://www.dropbox.com/s/r2ingd0l3zt8hxs/frozen_east_text_detection.tar.gz)
     * @node onnx_text_detection @alias onnxTextDetection
     * @param model — ONNX Text Detection Model
     * @param image — Input Image
     * @param threshold (optional) — Detection confidence threshold
     * @param inputSize (optional) — Model input size
     * @returns regions — Detected text regions
     * @returns count — Number of detected regions
     * @impure has side effects / drives control flow
     */
    function textDetection({ model: Struct, image: Struct, threshold?: float, inputSize?: int }): { regions: any, count: int };

    /**
     * Recognize text from cropped text regions. Download models from: CRNN (https://huggingface.co/Xenova/crnn_onnx), TrOCR (https://huggingface.co/microsoft/trocr-base-printed), PaddleOCR (https://huggingface.co/aapot/paddleocr-onnx)
     * @node onnx_text_recognition @alias onnxTextRecognition
     * @param model — ONNX Text Recognition Model
     * @param image — Cropped text region image
     * @param charset (optional) — Character set for decoding
     * @param inputHeight (optional) — Model expected input height
     * @returns result — Recognition result
     * @returns text — Recognized text string
     * @impure has side effects / drives control flow
     */
    function textRecognition({ model: Struct, image: Struct, charset?: string, inputHeight?: int }): { result: Struct, text: string };

    // === AI/ML/ONNX/Vision ===

    /**
     * Convert depth map to rainbow-colored visualization
     * @node depth_colorize @alias depthColorize
     * @param depthMap — Input depth map
     * @returns coloredImage — Rainbow-colored depth visualization
     * @impure has side effects / drives control flow
     */
    function depthColorize({ depthMap: Struct }): Struct;

    /**
     * Estimate depth from a single image using ONNX models. Download models from: MiDaS (https://github.com/isl-org/MiDaS/releases), DPT (https://huggingface.co/Intel/dpt-large/tree/main), Depth Anything (https://huggingface.co/depth-anything/Depth-Anything-V2-Small/tree/main)
     * @node onnx_depth_estimation @alias onnxDepthEstimation
     * @param model — ONNX Depth Model Session
     * @param image — Input Image
     * @param provider (optional) — Model provider type
     * @param inputSize (optional) — Model input size (default 384 for MiDaS)
     * @returns depthMap — Estimated depth map
     * @returns depthImage — Grayscale depth visualization
     * @impure has side effects / drives control flow
     */
    function depthEstimation({ model: Struct, image: Struct, provider?: Struct, inputSize?: int }): { depthMap: Struct, depthImage: Struct };

    /**
     * Convert depth map to 3D point cloud coordinates
     * @node depth_to_point_cloud @alias depthToPointCloud
     * @param depthMap — Input depth map
     * @param focalLength (optional) — Camera focal length (pixels)
     * @param scale (optional) — Depth scale factor
     * @returns points — 3D point coordinates [x, y, z]
     * @returns pointCount — Number of points
     * @impure has side effects / drives control flow
     */
    function depthToPointCloud({ depthMap: Struct, focalLength?: float, scale?: float }): { points: any, pointCount: int };
}

declare namespace tracking {
    // === AI/ML/Tracking ===

    /**
     * Gives local tracks from one or more cameras a global identity by comparing appearance embeddings. A track is collected for a few observations before it creates a new entity; later tracks that look like a known entity are matched to it, even on another camera. Every board and user of this app using the same task id shares the identities. They live in this process's memory only: after an hour without calls or a restart the task starts over and entity ids begin again at 1. An app can have at most 16 active tasks (64 per process); an idle task is released after an hour.
     * @node tracking_associate_entities @receiver observations @alias trackingAssociateEntities
     * @param observations (optional) — Observations with appearance embeddings, e.g. from Extract Appearance. Observations with a track id accumulate evidence per track; ones without are only matched to known entities, and lost tracks only report the entity they already have. Timestamps are Unix milliseconds from one clock shared by all cameras of the task; a missing timestamp means now, and one further ahead of this machine's clock than a minute (or half the entity lifetime, if shorter) is clamped. (receiver: `this` in `x.associateEntities(...)`)
     * @param taskId (optional) — Identities are shared by every board, user and camera of this app using the same task id
     * @param minSimilarity (optional) — Lowest cosine similarity for matching an observation to an entity
     * @param ambiguityMargin (optional) — A match is left unresolved when the next best entity is at most this much less similar
     * @param minObservations (optional) — Observations a new track needs before it is matched or creates an entity (at least 1)
     * @param entityTtlMs (optional) — Entities, track bindings and pending tracks unseen for this long, measured by observation timestamps, are forgotten
     * @returns associations — Observations with their global entity id, in input order
     * @returns unresolved — Observations without an entity yet, with the reason and the closest entities
     * @returns entityCount — Number of entities currently known to this task
     * @impure has side effects / drives control flow
     */
    function associateEntities(this: Struct[], { observations?: Struct[], taskId?: string, minSimilarity?: float, ambiguityMargin?: float, minObservations?: int, entityTtlMs?: int }): { associations: Struct[], unresolved: Struct[], entityCount: int };

    /**
     * Crops every detection from the frame and runs a re-identification model on the crops in batches. The built-in models are Intel OpenVINO person re-identification models (Apache-2.0): the chosen one is downloaded into Cache Dir on first use, checked against its SHA-256 and read from there afterwards. With Model set to custom, connect your own session from Load ONNX and set Normalization to match its export: imagenet for torchreid/OSNet exports, raw for FastReID onnx_export.py exports, which normalize inside the model. Each detection becomes an observation with an L2-normalized appearance embedding, keeping its box, camera, session, tracker id, track id and state (track-only fields such as hits and velocity are not carried). Lost tracks from Track Detections (Include Lost) carry a predicted box, so they are passed through with their existing embedding instead of being cropped. Feed the result into Track Detections for appearance-aware tracking or into Associate Entities to recognize the same object across cameras. Boxes that are not finite or smaller than Min Box Size are left out.
     * @node tracking_extract_appearance @receiver image_in @alias trackingExtractAppearance
     * @param model (optional) — person-openvino-0270 (6 MB, default) or person-openvino-0265 (9.6 MB): Intel OpenVINO person re-identification models, Apache-2.0, downloaded into Cache Dir on first use. custom uses the Custom Model pin.
     * @param cacheDir — Folder the built-in model is downloaded to when missing and loaded from when present. Not used with a custom model.
     * @param customModel — Session from Load ONNX, used when Model is custom. Takes an RGB float tensor [N,3,H,W] or [N,H,W,3] and returns one embedding per crop, shaped [N,D] or [N,D,1,1].
     * @param imageIn — Frame the detections were found in (receiver: `this` in `x.extractAppearance(...)`)
     * @param detections (optional) — Boxes in image pixel coordinates, from Object Detection or tracks from Track Detections
     * @param cameraId (optional) — Camera stamped on every observation; empty keeps the incoming value
     * @param sessionId (optional) — Camera session stamped on every observation; empty keeps the incoming value
     * @param timestampMs (optional) — Frame capture time in Unix milliseconds; 0 keeps the incoming value, or uses the current time when there is none
     * @param normalization (optional) — Pixel scaling a custom model expects (the built-in models normalize their input themselves, so any value works for them): imagenet ((x/255 - mean) / std) for torchreid/OSNet exports, raw (0–255 unchanged) for FastReID onnx_export.py exports that normalize inside the model, zero_one (x/255) or minus_one_one (x/127.5 - 1)
     * @param inputWidth (optional) — Crop width in pixels, used only when the model's input width is dynamic
     * @param inputHeight (optional) — Crop height in pixels, used only when the model's input height is dynamic
     * @param padding (optional) — Fraction of the box width and height added on each side before cropping
     * @param minBoxSize (optional) — Boxes narrower or shorter than this many pixels after padding and clipping to the image are skipped
     * @param batchSize (optional) — Crops per inference call, used only when the model's batch dimension is dynamic
     * @returns observations — One observation per cropped detection with its new embedding, plus every lost track with its existing one; each carries its index in Detections
     * @returns dimensions — Embedding length of the model output; 0 when no detection was cropped
     * @impure has side effects / drives control flow
     */
    function extractAppearance(this: NodeImage, { model?: string, cacheDir: Struct, customModel: Struct, imageIn: Struct, detections?: Struct[], cameraId?: string, sessionId?: string, timestampMs?: int, normalization?: string, inputWidth?: int, inputHeight?: int, padding?: float, minBoxSize?: float, batchSize?: int }): { observations: Struct[], dimensions: int };

    /**
     * Follows detected objects across the frames of one camera and gives each a stable track id (ByteTrack, with BoT-SORT appearance matching when detections carry embeddings). Run it once per frame: the tracker is kept in memory per user, board, node, Camera pin and Session pin between runs and uses frame timestamps for motion and expiry. Track ids are only unique per tracker instance, named by each track's tracker_id: a new Camera or Session pin value, 10 minutes without frames, eviction when too many trackers are open, or a process restart starts a new tracker with a new tracker_id (a session carried by the detections does not). Deployments that spread runs over several processes keep one independent tracker per process.
     * @node tracking_track_detections @receiver detections @alias trackingTrackDetections
     * @param detections (optional) — Boxes detected in this frame. Embeddings from Extract Appearance are used for appearance matching when present. Elements in the lost state (predicted boxes of lost tracks fed back in) are ignored. (receiver: `this` in `x.trackDetections(...)`)
     * @param cameraId (optional) — Camera the frame comes from. Empty keeps the camera carried by the detections, or the last one seen. Every value of this pin gets its own tracker, so set it when one node tracks several cameras.
     * @param sessionId (optional) — Stream session of the camera. Empty keeps the session carried by the detections, or the last one seen. A new value of this pin starts a fresh tracker with a new tracker_id.
     * @param timestampMs (optional) — Frame capture time in Unix milliseconds; 0 uses the current time, never earlier than the last processed frame. A frame up to Max Lost older than the last processed one is skipped; one further back is taken as a clock reset that drops all tracks.
     * @param highThreshold (optional) — Detections scoring at least this take part in the first association
     * @param lowThreshold (optional) — Detections scoring at least this, but below the high threshold, can only keep existing tracks alive
     * @param newTrackThreshold (optional) — Minimum score for an unmatched detection to start a new track
     * @param matchThreshold (optional) — Maximum matching cost (1 - IoU × score, or appearance distance) of the first association; higher matches more loosely
     * @param maxLostMs (optional) — How long a track that lost its object can be re-acquired with the same id
     * @param minAppearanceSimilarity (optional) — Minimum cosine similarity of embeddings for appearance to count as a match
     * @param classAware (optional) — Never match a track with a detection of another class
     * @param includeLost (optional) — Also output tracks that lost their object. Their boxes are Kalman predictions, not detections: Extract Appearance and Associate Entities treat them as passive, and this node ignores them when they are fed back in.
     * @returns tracks — Confirmed tracks matched in this frame (plus lost tracks if enabled), sorted by track id. Track ids are unique per tracker_id.
     * @returns skipped — True when the frame was older than the last processed one and was ignored
     * @impure has side effects / drives control flow
     */
    function trackDetections(this: Struct[], { detections?: Struct[], cameraId?: string, sessionId?: string, timestampMs?: int, highThreshold?: float, lowThreshold?: float, newTrackThreshold?: float, matchThreshold?: float, maxLostMs?: int, minAppearanceSimilarity?: float, classAware?: bool, includeLost?: bool }): { tracks: Struct[], skipped: bool };
}
