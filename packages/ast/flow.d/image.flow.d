// Image — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Image ===

/**
 * Decode a still image and write it as PNG, JPEG, GIF, WebP, or AVIF
 * @param source — Source image FlowPath
 * @param target — Target image FlowPath
 * @param format (optional) — Output image format, or auto from target extension
 * @returns result — Written image FlowPath
 * @returns report — Image conversion report
 * @impure has side effects / drives control flow
 */
declare function videoConvertImageFormat({ source: Struct, target: Struct, format?: string }): { result: Struct, report: Struct };

/**
 * Apply crop, resize, flip, rotate, blur, and color filters to a still image
 * @param source — Source image FlowPath
 * @param target — Target image FlowPath
 * @param format (optional) — Output image format, or auto from target extension
 * @param cropX (optional)
 * @param cropY (optional)
 * @param cropWidth (optional)
 * @param cropHeight (optional)
 * @param resizeWidth (optional)
 * @param resizeHeight (optional)
 * @param rotateDegrees (optional)
 * @param blurRadius (optional)
 * @param flipHorizontal (optional)
 * @param flipVertical (optional)
 * @param brightness (optional) — -1.0 to 1.0
 * @param contrast (optional) — 1.0 keeps contrast unchanged
 * @param saturation (optional) — 1.0 keeps saturation unchanged
 * @returns result — Written image FlowPath
 * @returns report — Image transform report
 * @impure has side effects / drives control flow
 */
declare function videoTransformImage({ source: Struct, target: Struct, format?: string, cropX?: int, cropY?: int, cropWidth?: int, cropHeight?: int, resizeWidth?: int, resizeHeight?: int, rotateDegrees?: int, blurRadius?: int, flipHorizontal?: bool, flipVertical?: bool, brightness?: float, contrast?: float, saturation?: float }): { result: Struct, report: Struct };


// === Image/Annotate ===

/**
 * Draw Bounding Boxes
 * @param imageIn — Image object
 * @param bboxes — Bounding Boxes
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Bounding Boxes
 * @impure has side effects / drives control flow
 */
declare function drawBoxes({ imageIn: Struct, bboxes: Struct[], useRef?: bool }): Struct;

/**
 * Make Bounding Box
 * @param definition (optional) — Bounding Box Definition
 * @param classIdx (optional) — Class Index
 * @param score (optional) — Score or Confidence
 * @param x1 — Left
 * @param y1 — Top
 * @param x2 — Right
 * @param y2 — Bottom
 * @returns bbox — Bounding Boxes
 * @impure has side effects / drives control flow
 */
declare function makeBoxe({ definition?: string, classIdx?: int, score?: float, x1: float, y1: float, x2: float, y2: float }): Struct;


// === Image/Content ===

/**
 * Read/Decode QR Codes and Barcodes
 * @param imageIn — Image object
 * @param options (optional) — Barcode decoding options
 * @returns results — Detected/Decoded Codes
 * @impure has side effects / drives control flow
 */
declare function readBarcodes({ imageIn: Struct, options?: Struct }): Struct[];

/**
 * Read image from path
 * @param path — FlowPath
 * @param applyExif (optional) — Apply Exif Orientation
 * @returns imageOut — Image object
 * @impure has side effects / drives control flow
 */
declare function readImage({ path: Struct, applyExif?: bool }): Struct;

/**
 * Read image from path
 * @param signedUrl — Signed Url
 * @param applyExif (optional) — Apply Exif Orientation
 * @returns imageOut — Image object
 * @impure has side effects / drives control flow
 */
declare function readImageUrl({ signedUrl: string, applyExif?: bool }): Struct;

/**
 * Write image to path
 * @param imageIn — The image to write to path
 * @param path — FlowPath
 * @param type (optional) — Image Type
 * @param quality (optional) — Encoding Quality
 * @impure has side effects / drives control flow
 */
declare function writeImage({ imageIn: Struct, path: Struct, type?: string, quality?: int }): void;


// === Image/Metadata ===

/**
 * Get Image Dimensions
 * @param imageIn — Image object
 * @returns width — Image Width
 * @returns height — Image Height
 * @impure has side effects / drives control flow
 */
declare function getDimensions({ imageIn: Struct }): { width: int, height: int };


// === Image/Overlay ===

/**
 * Overlay one image on top of another with configurable position, size, opacity and fit mode
 * @param baseImage — The background image
 * @param overlayImage — The image to overlay on top
 * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
 * @param x (optional) — Horizontal offset in pixels from the left edge
 * @param y (optional) — Vertical offset in pixels from the top edge
 * @param maxW (optional) — Maximum width of the overlay (0 = original width)
 * @param maxH (optional) — Maximum height of the overlay (0 = original height)
 * @param opacity (optional) — Overlay opacity from 0.0 (transparent) to 1.0 (opaque)
 * @param fitMode (optional) — How to fit the overlay into max width/height
 * @returns imageOut — Result image with overlay applied
 * @impure has side effects / drives control flow
 */
declare function imageOverlay({ baseImage: Struct, overlayImage: Struct, useRef?: bool, x?: int, y?: int, maxW?: int, maxH?: int, opacity?: float, fitMode?: string }): Struct;

/**
 * Draw text on top of an image with configurable font size, position, and color
 * @param baseImage — The background image to draw text on
 * @param text (optional) — The text string to render
 * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
 * @param x (optional) — Horizontal offset in pixels from the left edge
 * @param y (optional) — Vertical offset in pixels from the top edge
 * @param fontSize (optional) — Font size in pixels
 * @param color (optional) — Text color as hex string (e.g. #FF0000 or #FF0000AA for alpha)
 * @returns imageOut — Result image with text rendered
 * @impure has side effects / drives control flow
 */
declare function textOverlay({ baseImage: Struct, text?: string, useRef?: bool, x?: int, y?: int, fontSize?: float, color?: string }): Struct;


// === Image/PDF ===

/**
 * Count pages in a PDF
 * @param pdf — PDF file
 * @returns pageCount — Page count
 * @impure has side effects / drives control flow
 */
declare function pdfPageCount({ pdf: Struct }): int;

/**
 * Render a single PDF page as an image
 * @param pdf — PDF file
 * @param page (optional) — 1-based page number
 * @param scale (optional) — Render scale
 * @param bgColor (optional) — Background color for the rendered page
 * @returns image — Rendered image
 * @impure has side effects / drives control flow
 */
declare function pdfPageToImage({ pdf: Struct, page?: int, scale?: float, bgColor?: string }): Struct;

/**
 * Render every PDF page as an ordered image array
 * @param pdf — PDF file
 * @param scale (optional) — Render scale
 * @param bgColor (optional) — Background color for rendered pages
 * @returns images — Rendered images
 * @impure has side effects / drives control flow
 */
declare function pdfToImages({ pdf: Struct, scale?: float, bgColor?: string }): Struct[];


// === Image/Transform ===

/**
 * Adjust Image Contrast
 * @param imageIn — Image object
 * @param contrast — Contrast
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Applied Contrast
 * @impure has side effects / drives control flow
 */
declare function contrastImage({ imageIn: Struct, contrast: float, useRef?: bool }): Struct;

/**
 * Convert Image Color/Pixel Type (e.g. to Grayscale)
 * @param imageIn — Image object
 * @param pixelType (optional) — Target Pixel Type
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Target Color/Pixel Type
 * @impure has side effects / drives control flow
 */
declare function convertImage({ imageIn: Struct, pixelType?: string, useRef?: bool }): Struct;

/**
 * Crop Image
 * @param imageIn — Image object
 * @param bbox — Bounding Box
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Cropped Image object
 * @impure has side effects / drives control flow
 */
declare function cropImage({ imageIn: Struct, bbox: Struct, useRef?: bool }): Struct;

/**
 * Resize Image
 * @param imageIn — Image object
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @param mode (optional) — Resize Mode
 * @param filter (optional) — Resize Filter Algorithm
 * @param widthIn (optional) — Resized Image Target Width
 * @param heightIn (optional) — Resized Image Target Height
 * @returns imageOut — Image object
 * @returns widthOut — Resized Image Result Width
 * @returns heightOut — Resized Image Result Height
 * @impure has side effects / drives control flow
 */
declare function resizeImage({ imageIn: Struct, useRef?: bool, mode?: string, filter?: string, widthIn?: int, heightIn?: int }): { imageOut: Struct, widthOut: int, heightOut: int };

