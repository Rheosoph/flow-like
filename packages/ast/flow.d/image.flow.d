// Image — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace image {
    // === Image ===

    /**
     * Decode a still image and write it as PNG, JPEG, GIF, WebP, or AVIF
     * @node video_convert_image_format @alias videoConvertImageFormat
     * @param source — Source image FlowPath
     * @param target — Target image FlowPath
     * @param format (optional) — Output image format, or auto from target extension
     * @returns result — Written image FlowPath
     * @returns report — Image conversion report
     * @impure has side effects / drives control flow
     */
    function convertFormat({ source: Struct, target: Struct, format?: string }): { result: Struct, report: Struct };

    /**
     * Apply crop, resize, flip, rotate, blur, and color filters to a still image
     * @node video_transform_image @alias videoTransformImage
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
    function transform({ source: Struct, target: Struct, format?: string, cropX?: int, cropY?: int, cropWidth?: int, cropHeight?: int, resizeWidth?: int, resizeHeight?: int, rotateDegrees?: int, blurRadius?: int, flipHorizontal?: bool, flipVertical?: bool, brightness?: float, contrast?: float, saturation?: float }): { result: Struct, report: Struct };

    // === Image/Annotate ===

    /**
     * Draw Bounding Boxes
     * @node draw_boxes @receiver image_in @alias drawBoxes
     * @param imageIn — Image object (receiver: `this` in `x.drawBoxes(...)`)
     * @param bboxes — Bounding Boxes
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Bounding Boxes
     * @impure has side effects / drives control flow
     */
    function drawBoxes(this: NodeImage, { imageIn: Struct, bboxes: Struct[], useRef?: bool }): Struct;

    /**
     * Make Bounding Box
     * @node make_boxe @alias makeBoxe
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
    function makeBox({ definition?: string, classIdx?: int, score?: float, x1: float, y1: float, x2: float, y2: float }): Struct;

    // === Image/Content ===

    /**
     * Read image from path
     * @node read_image @alias readImage
     * @param path — FlowPath
     * @param applyExif (optional) — Apply Exif Orientation
     * @returns imageOut — Image object
     * @impure has side effects / drives control flow
     */
    function read({ path: Struct, applyExif?: bool }): Struct;

    /**
     * Read/Decode QR Codes and Barcodes
     * @node read_barcodes @receiver image_in @alias readBarcodes
     * @param imageIn — Image object (receiver: `this` in `x.readBarcodes(...)`)
     * @param options (optional) — Barcode decoding options
     * @returns results — Detected/Decoded Codes
     * @impure has side effects / drives control flow
     */
    function readBarcodes(this: NodeImage, { imageIn: Struct, options?: Struct }): Struct[];

    /**
     * Read image from path
     * @node read_image_url @alias readImageUrl
     * @param signedUrl — Signed Url
     * @param applyExif (optional) — Apply Exif Orientation
     * @returns imageOut — Image object
     * @impure has side effects / drives control flow
     */
    function readUrl({ signedUrl: string, applyExif?: bool }): Struct;

    /**
     * Write image to path
     * @node write_image @receiver image_in @alias writeImage
     * @param imageIn — The image to write to path (receiver: `this` in `x.write(...)`)
     * @param path — FlowPath
     * @param type (optional) — Image Type
     * @param quality (optional) — Encoding Quality
     * @impure has side effects / drives control flow
     */
    function write(this: NodeImage, { imageIn: Struct, path: Struct, type?: string, quality?: int }): void;

    // === Image/Metadata ===

    /**
     * Get Image Dimensions
     * @node get_dimensions @receiver image_in @alias getDimensions
     * @param imageIn — Image object (receiver: `this` in `x.getDimensions(...)`)
     * @returns width — Image Width
     * @returns height — Image Height
     * @impure has side effects / drives control flow
     */
    function getDimensions(this: NodeImage, { imageIn: Struct }): { width: int, height: int };

    // === Image/Overlay ===

    /**
     * Overlay one image on top of another with configurable position, size, opacity and fit mode
     * @node image_overlay @receiver base_image @alias imageOverlay
     * @param baseImage — The background image (receiver: `this` in `x.overlay(...)`)
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
    function overlay(this: NodeImage, { baseImage: Struct, overlayImage: Struct, useRef?: bool, x?: int, y?: int, maxW?: int, maxH?: int, opacity?: float, fitMode?: string }): Struct;

    /**
     * Draw text on top of an image with configurable font size, position, and color
     * @node text_overlay @receiver base_image @alias textOverlay
     * @param baseImage — The background image to draw text on (receiver: `this` in `x.textOverlay(...)`)
     * @param text (optional) — The text string to render
     * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
     * @param x (optional) — Horizontal offset in pixels from the left edge
     * @param y (optional) — Vertical offset in pixels from the top edge
     * @param fontSize (optional) — Font size in pixels
     * @param color (optional) — Text color as hex string (e.g. #FF0000 or #FF0000AA for alpha)
     * @returns imageOut — Result image with text rendered
     * @impure has side effects / drives control flow
     */
    function textOverlay(this: NodeImage, { baseImage: Struct, text?: string, useRef?: bool, x?: int, y?: int, fontSize?: float, color?: string }): Struct;

    // === Image/Transform ===

    /**
     * Adjust Image Contrast
     * @node contrast_image @receiver image_in @alias contrastImage
     * @param imageIn — Image object (receiver: `this` in `x.contrast(...)`)
     * @param contrast — Contrast
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Applied Contrast
     * @impure has side effects / drives control flow
     */
    function contrast(this: NodeImage, { imageIn: Struct, contrast: float, useRef?: bool }): Struct;

    /**
     * Convert Image Color/Pixel Type (e.g. to Grayscale)
     * @node convert_image @receiver image_in @alias convertImage
     * @param imageIn — Image object (receiver: `this` in `x.convertColor(...)`)
     * @param pixelType (optional) — Target Pixel Type
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Target Color/Pixel Type
     * @impure has side effects / drives control flow
     */
    function convertColor(this: NodeImage, { imageIn: Struct, pixelType?: string, useRef?: bool }): Struct;

    /**
     * Crop Image
     * @node crop_image @receiver image_in @alias cropImage
     * @param imageIn — Image object (receiver: `this` in `x.crop(...)`)
     * @param bbox — Bounding Box
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Cropped Image object
     * @impure has side effects / drives control flow
     */
    function crop(this: NodeImage, { imageIn: Struct, bbox: Struct, useRef?: bool }): Struct;

    /**
     * Resize Image
     * @node resize_image @receiver image_in @alias resizeImage
     * @param imageIn — Image object (receiver: `this` in `x.resize(...)`)
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
    function resize(this: NodeImage, { imageIn: Struct, useRef?: bool, mode?: string, filter?: string, widthIn?: int, heightIn?: int }): { imageOut: Struct, widthOut: int, heightOut: int };
}

declare namespace pdf {
    // === Image/PDF ===

    /**
     * Count pages in a PDF
     * @node pdf_page_count @alias pdfPageCount
     * @param pdf — PDF file
     * @returns pageCount — Page count
     * @impure has side effects / drives control flow
     */
    function pageCount({ pdf: Struct }): int;

    /**
     * Render a single PDF page as an image
     * @node pdf_page_to_image @alias pdfPageToImage
     * @param pdf — PDF file
     * @param page (optional) — 1-based page number
     * @param scale (optional) — Render scale
     * @param bgColor (optional) — Background color for the rendered page
     * @returns image — Rendered image
     * @impure has side effects / drives control flow
     */
    function pageToImage({ pdf: Struct, page?: int, scale?: float, bgColor?: string }): Struct;

    /**
     * Render every PDF page as an ordered image array
     * @node pdf_to_images @alias pdfToImages
     * @param pdf — PDF file
     * @param scale (optional) — Render scale
     * @param bgColor (optional) — Background color for rendered pages
     * @returns images — Rendered images
     * @impure has side effects / drives control flow
     */
    function toImages({ pdf: Struct, scale?: float, bgColor?: string }): Struct[];
}
