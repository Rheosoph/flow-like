// Document — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace docx {
    // === Document/DOCX ===

    /**
     * Set header and/or footer text in a DOCX document
     * @node docx_add_header_footer @alias docxAddHeaderFooter
     * @param template — DOCX file
     * @param headerText (optional) — Text for the header
     * @param footerText (optional) — Text for the footer
     * @param includePageNumber (optional) — Add page number to footer
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addHeaderFooter({ template: Struct, headerText?: string, footerText?: string, includePageNumber?: bool, output: Struct }): Struct;

    /**
     * Append a hyperlink to a DOCX document. Default color: #FF4343.
     * @node docx_add_hyperlink @alias docxAddHyperlink
     * @param template — DOCX file
     * @param displayText — Visible link text
     * @param url — Hyperlink URL
     * @param fontColor (optional) — Link color (hex)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addHyperlink({ template: Struct, displayText: string, url: string, fontColor?: string, output: Struct }): Struct;

    /**
     * Insert an inline image into a DOCX document
     * @node docx_add_image @alias docxAddImage
     * @param template — DOCX file
     * @param image — Image file to insert
     * @param widthCm (optional) — Image width in cm
     * @param heightCm (optional) — Image height in cm
     * @param altText (optional) — Accessibility alt text
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addImage({ template: Struct, image: Struct, widthCm?: float, heightCm?: float, altText?: string, output: Struct }): Struct;

    /**
     * Insert a page break into a DOCX document
     * @node docx_add_page_break @alias docxAddPageBreak
     * @param template — DOCX file
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addPageBreak({ template: Struct, output: Struct }): Struct;

    /**
     * Append a styled paragraph to a DOCX document
     * @node docx_add_paragraph @alias docxAddParagraph
     * @param template — DOCX file to append to
     * @param text — Paragraph text (supports markdown bold/italic)
     * @param style (optional) — Paragraph style: Normal, Heading1-6, Title, Subtitle, Quote
     * @param fontFamily (optional) — Override font
     * @param fontSize (optional) — Override size in points (0 = use style default)
     * @param fontColor (optional) — Override text color (hex)
     * @param bold (optional) — Force bold
     * @param italic (optional) — Force italic
     * @param alignment (optional) — Text alignment: Left, Center, Right, Justify
     * @param output — Where to save
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addParagraph({ template: Struct, text: string, style?: string, fontFamily?: string, fontSize?: float, fontColor?: string, bold?: bool, italic?: bool, alignment?: string, output: Struct }): Struct;

    /**
     * Insert a styled table from JSON data. Default: branded header with #FF4343, zebra rows.
     * @node docx_add_table @alias docxAddTable
     * @param template — DOCX file to add table to
     * @param data — JSON array of arrays (first row = headers if header_row=true)
     * @param headerRow (optional) — Style first row as header
     * @param alternateRows (optional) — Zebra striping
     * @param borderColor (optional) — Table border color (hex)
     * @param fontSize (optional) — Font size in points
     * @param output — Where to save
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addTable({ template: Struct, data: string, headerRow?: bool, alternateRows?: bool, borderColor?: string, fontSize?: float, output: Struct }): Struct;

    /**
     * Insert a TOC field that Word will populate on open
     * @node docx_add_toc @alias docxAddToc
     * @param template — DOCX file
     * @param title (optional) — TOC title
     * @param maxLevel (optional) — Maximum heading level to include (1-6)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addToc({ template: Struct, title?: string, maxLevel?: int, output: Struct }): Struct;

    /**
     * Create an empty DOCX with Flow Like branded theme (styled headings, Calibri font, modern spacing)
     * @node docx_create @alias docxCreate
     * @param fontFamily (optional) — Default body font
     * @param fontSize (optional) — Body font size in points
     * @param themeColor (optional) — Accent color for headings (hex)
     * @param output — Where to save the DOCX file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function create({ fontFamily?: string, fontSize?: float, themeColor?: string, output: Struct }): Struct;

    /**
     * Extract all text content from a DOCX file as plain text
     * @node docx_extract_text @alias docxExtractText
     * @param template — DOCX file to extract from
     * @returns text — Extracted text content
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Read document metadata from docProps/core.xml
     * @node docx_get_metadata @alias docxGetMetadata
     * @param template — DOCX file
     * @returns title — Document title
     * @returns author — Document author
     * @returns subject — Document subject
     * @returns keywords — Document keywords
     * @returns description — Document description
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, description: string };

    /**
     * Scan document body, headers, footers for all {{...}} placeholder strings
     * @node docx_list_placeholders @alias docxListPlaceholders
     * @param template — DOCX template file
     * @returns placeholders — List of placeholder strings found
     * @impure has side effects / drives control flow
     */
    function listPlaceholders({ template: Struct }): string[];

    /**
     * Concatenate multiple DOCX documents into one, with optional page breaks between them
     * @node docx_merge @alias docxMerge
     * @param documents — Array of DOCX file paths to merge in order
     * @param pageBreak (optional) — Insert page break between documents
     * @param output — Where to save the merged file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ documents: Struct[], pageBreak?: bool, output: Struct }): Struct;

    /**
     * Remove paragraphs containing a specific placeholder. Useful for conditional content.
     * @node docx_remove_paragraph @alias docxRemoveParagraph
     * @param template — DOCX file to modify
     * @param placeholder — Text to search for — paragraphs containing this are removed
     * @param output — Where to save the result
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function removeParagraph({ template: Struct, placeholder: string, output: Struct }): Struct;

    /**
     * Replace an image in a DOCX file by matching alt text or shape name
     * @node docx_replace_image @alias docxReplaceImage
     * @param template — DOCX template file
     * @param identifier — Alt text or shape name of the image to replace
     * @param image — Replacement image file
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Where to save the resulting DOCX file
     * @returns result — Path to the output file
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Find a table containing a placeholder, duplicate that row for each data item, replacing placeholders per row
     * @node docx_replace_table_row @alias docxReplaceTableRow
     * @param template — DOCX template file
     * @param placeholder — Placeholder in the template row (e.g. {{item}})
     * @param data — JSON array of objects — each object's keys match placeholders in the row
     * @param output — Where to save the result
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function replaceTableRow({ template: Struct, placeholder: string, data: string, output: Struct }): Struct;

    /**
     * Replace text placeholders in a DOCX template file with plain text or markdown
     * @node docx_replace_text @alias docxReplaceText
     * @param template — DOCX template file
     * @param placeholder — Placeholder text to find (e.g. {{name}})
     * @param replacement — Replacement text (supports markdown when enabled)
     * @param useMarkdown (optional) — Parse the replacement text as markdown
     * @param output — Where to save the resulting DOCX file
     * @returns result — Path to the output file
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

    /**
     * Set title, author, subject, keywords, description in document metadata
     * @node docx_set_metadata @alias docxSetMetadata
     * @param template — DOCX file
     * @param title (optional) — Document title
     * @param author (optional) — Document author
     * @param subject (optional) — Document subject
     * @param keywords (optional) — Keywords
     * @param description (optional) — Document description
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, description?: string, output: Struct }): Struct;
}

declare namespace pdf {
    // === Document/PDF ===

    /**
     * Stamp an image at a specified position on selected PDF pages.
     * @node pdf_add_image_stamp @alias pdfAddImageStamp
     * @param template — PDF file
     * @param image — Image file (PNG/JPEG)
     * @param x (optional) — X position in points
     * @param y (optional) — Y position in points
     * @param width (optional) — Image width in points
     * @param height (optional) — Image height in points
     * @param pages (optional) — Page numbers (empty = all)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addImageStamp({ template: Struct, image: Struct, x?: float, y?: float, width?: float, height?: float, pages?: int[], output: Struct }): Struct;

    /**
     * Add 'Page X of Y' labels to each page of a PDF.
     * @node pdf_add_page_numbers @alias pdfAddPageNumbers
     * @param template — PDF file
     * @param position (optional) — Position: bottom-center, bottom-right, bottom-left
     * @param fontSize (optional) — Font size in points
     * @param margin (optional) — Margin from edge in points
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addPageNumbers({ template: Struct, position?: string, fontSize?: float, margin?: float, output: Struct }): Struct;

    /**
     * Overlay a diagonal text watermark on all pages. Default: #FF4343 at 15% opacity.
     * @node pdf_add_watermark @alias pdfAddWatermark
     * @param template — PDF file
     * @param text — Watermark text
     * @param fontSize (optional) — Font size in points
     * @param color (optional) — Watermark color (hex)
     * @param opacity (optional) — 0.0 to 1.0
     * @param rotationDeg (optional) — Rotation in degrees
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addWatermark({ template: Struct, text: string, fontSize?: float, color?: string, opacity?: float, rotationDeg?: float, output: Struct }): Struct;

    /**
     * Optimize and compress a PDF by deduplicating objects and compressing streams.
     * @node pdf_compress @alias pdfCompress
     * @param template — PDF file
     * @param output — Save path
     * @returns result — Output file path
     * @returns originalSize — Size in bytes before compression
     * @returns compressedSize — Size in bytes after compression
     * @impure has side effects / drives control flow
     */
    function compress({ template: Struct, output: Struct }): { result: Struct, originalSize: int, compressedSize: int };

    /**
     * Remove password protection from a PDF using the owner or user password.
     * @node pdf_decrypt @alias pdfDecrypt
     * @param template — Encrypted PDF file
     * @param password — Owner or user password
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function decrypt({ template: Struct, password: string, output: Struct }): Struct;

    /**
     * Encrypt a PDF with a user password for restricted access.
     * @node pdf_encrypt @alias pdfEncrypt
     * @param template — PDF file
     * @param userPassword — Password required to open
     * @param ownerPassword (optional) — Password for full access (optional, defaults to user password)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function encrypt({ template: Struct, userPassword: string, ownerPassword?: string, output: Struct }): Struct;

    /**
     * Extract specific pages (non-contiguous) from a PDF
     * @node pdf_extract_pages @alias pdfExtractPages
     * @param template — PDF file
     * @param pages — Array of page numbers to extract (1-based)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function extractPages({ template: Struct, pages: int[], output: Struct }): Struct;

    /**
     * Extract all text content from a PDF document.
     * @node pdf_extract_text @alias pdfExtractText
     * @param template — PDF file
     * @returns text — Extracted text
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Sets the value of a named AcroForm field in a PDF document.
     * @node pdf_fill_form @alias pdfFillForm
     * @param template — PDF file containing form fields
     * @param fieldName — Name of the AcroForm field to fill
     * @param fieldValue — Value to set on the form field
     * @param output — Path to save the filled PDF
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function fillForm({ template: Struct, fieldName: string, fieldValue: string, output: Struct }): Struct;

    /**
     * Typesets Markdown into a paginated PDF with selectable text, tables, code blocks, charts and embedded images
     * @node pdf_create_from_markdown @alias pdfCreateFromMarkdown
     * @param markdown — Markdown source to typeset
     * @param output — Save path
     * @param pageSize (optional) — Page geometry
     * @param embedImages (optional) — Download and embed images referenced by the Markdown. Disable to render placeholders instead.
     * @param pageNumbers (optional) — Print a page number in the footer
     * @param title (optional) — Document title. Also sets the running header and the cover block.
     * @param subtitle (optional) — Secondary line under the title on the cover block
     * @param cover (optional) — Open the document with the accent title block
     * @param author (optional) — Document author metadata
     * @returns result — Output file path
     * @returns pages — Number of pages written
     * @impure has side effects / drives control flow
     */
    function fromMarkdown({ markdown: string, output: Struct, pageSize?: string, embedImages?: bool, pageNumbers?: bool, title?: string, subtitle?: string, cover?: bool, author?: string }): { result: Struct, pages: int };

    /**
     * Read title, author, subject, keywords, and page count from a PDF.
     * @node pdf_get_metadata @alias pdfGetMetadata
     * @param template — PDF file
     * @returns title — Document title
     * @returns author — Author
     * @returns subject — Subject
     * @returns keywords — Keywords
     * @returns pageCount — Number of pages
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, pageCount: int };

    /**
     * Reads a PDF and returns all AcroForm field names so you know which fields are available to fill.
     * @node pdf_list_form_fields @alias pdfListFormFields
     * @param template — PDF file containing form fields
     * @returns fieldNames — Array of all form field names in the PDF
     * @returns fieldCount — Total number of form fields
     * @impure has side effects / drives control flow
     */
    function listFormFields({ template: Struct }): { fieldNames: string[], fieldCount: int };

    /**
     * Concatenate multiple PDF files into one
     * @node pdf_merge @alias pdfMerge
     * @param documents — Array of PDF file paths to merge in order
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ documents: Struct[], output: Struct }): Struct;

    /**
     * Replaces an image XObject in a PDF by name. Any image format is accepted and automatically converted to JPEG.
     * @node pdf_replace_image @alias pdfReplaceImage
     * @param template — PDF file containing the image to replace
     * @param imageName — XObject image name (e.g. "Im0", "Image1")
     * @param image — Replacement image file (any format — auto-converted to JPEG)
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Path to save the modified PDF
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, imageName: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Attempts to find and replace text in a PDF. Best-effort: PDF text replacement may not work for all documents due to complex text encoding and fragmented content streams.
     * @node pdf_replace_text @alias pdfReplaceText
     * @param template — PDF file to modify
     * @param placeholder — Text to find in the PDF
     * @param replacement — Plain text replacement value
     * @param output — Path to save the modified PDF
     * @returns result — Output file path
     * @returns replacedCount — Number of text replacements made
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, output: Struct }): { result: Struct, replacedCount: int };

    /**
     * Rotate pages by 90, 180, or 270 degrees
     * @node pdf_rotate_pages @alias pdfRotatePages
     * @param template — PDF file
     * @param pages (optional) — Page numbers to rotate (1-based). Empty array = all pages.
     * @param rotation (optional) — Rotation degrees: 90, 180, or 270
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function rotatePages({ template: Struct, pages?: int[], rotation?: int, output: Struct }): Struct;

    /**
     * Set title, author, subject, and keywords in a PDF's Info dictionary.
     * @node pdf_set_metadata @alias pdfSetMetadata
     * @param template — PDF file
     * @param title (optional) — Document title
     * @param author (optional) — Author
     * @param subject (optional) — Subject
     * @param keywords (optional) — Keywords
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

    /**
     * Extract a page range from a PDF into a new file
     * @node pdf_split @alias pdfSplit
     * @param template — PDF file
     * @param startPage (optional) — First page to extract (1-based)
     * @param endPage (optional) — Last page to extract (1-based, inclusive)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function split({ template: Struct, startPage?: int, endPage?: int, output: Struct }): Struct;
}

declare namespace pptx {
    // === Document/PPTX ===

    /**
     * Embed a simple bar chart on a PPTX slide using DrawingML chart XML.
     * @node pptx_add_chart @alias pptxAddChart
     * @param template — PPTX file
     * @param slideNumber (optional) — 1-based slide index
     * @param chartType (optional) — bar, line, or pie
     * @param categories — Category labels
     * @param values — Numeric values
     * @param seriesName (optional) — Series label
     * @param x (optional) — X position in cm
     * @param y (optional) — Y position in cm
     * @param width (optional) — Width in cm
     * @param height (optional) — Height in cm
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addChart({ template: Struct, slideNumber?: int, chartType?: string, categories: string[], values: float[], seriesName?: string, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

    /**
     * Place an image at a specified position on a PPTX slide.
     * @node pptx_add_image_to_slide @alias pptxAddImageToSlide
     * @param template — PPTX file
     * @param image — Image file (PNG/JPEG)
     * @param slideNumber (optional) — 1-based slide index
     * @param x (optional) — X position in cm
     * @param y (optional) — Y position in cm
     * @param width (optional) — Width in cm
     * @param height (optional) — Height in cm
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addImage({ template: Struct, image: Struct, slideNumber?: int, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

    /**
     * Set or replace speaker notes for a slide
     * @node pptx_add_notes @alias pptxAddNotes
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Which slide to set notes for (1-based)
     * @param notes — Speaker notes text
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function addNotes({ template: Struct, slideIndex?: int, notes: string, output: Struct }): Struct;

    /**
     * Add a shape (rectangle, ellipse, arrow, etc.) to a PPTX slide.
     * @node pptx_add_shape @alias pptxAddShape
     * @param template — PPTX file
     * @param slideNumber (optional) — 1-based slide index
     * @param shape (optional) — Shape preset: rect, ellipse, roundRect, rightArrow, diamond, triangle
     * @param x (optional) — X position in cm
     * @param y (optional) — Y position in cm
     * @param width (optional) — Width in cm
     * @param height (optional) — Height in cm
     * @param fillColor (optional) — Fill hex color
     * @param lineColor (optional) — Outline hex color (empty = no outline)
     * @param text (optional) — Optional text inside shape
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addShape({ template: Struct, slideNumber?: int, shape?: string, x?: float, y?: float, width?: float, height?: float, fillColor?: string, lineColor?: string, text?: string, output: Struct }): Struct;

    /**
     * Add a blank slide to a PPTX presentation.
     * @node pptx_add_slide @alias pptxAddSlide
     * @param template — PPTX file
     * @param output — Save path
     * @returns result — Output file path
     * @returns slideNumber — New slide's index (1-based)
     * @impure has side effects / drives control flow
     */
    function addSlide({ template: Struct, output: Struct }): { result: Struct, slideNumber: int };

    /**
     * Add a branded table to a PPTX slide. Header row uses #FF4343 with white text.
     * @node pptx_add_table_to_slide @alias pptxAddTableToSlide
     * @param template — PPTX file
     * @param slideNumber (optional) — 1-based slide index
     * @param headers — Column headers
     * @param rows — Table data as JSON array of arrays
     * @param x (optional) — X position in cm
     * @param y (optional) — Y position in cm
     * @param width (optional) — Table width in cm
     * @param rowHeight (optional) — Row height in cm
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addTable({ template: Struct, slideNumber?: int, headers: string[], rows: string[], x?: float, y?: float, width?: float, rowHeight?: float, output: Struct }): Struct;

    /**
     * Add a styled text box to a specific slide in a PPTX.
     * @node pptx_add_text_box @alias pptxAddTextBox
     * @param template — PPTX file
     * @param slideNumber (optional) — 1-based slide index
     * @param text — Text content
     * @param x (optional) — X position in cm
     * @param y (optional) — Y position in cm
     * @param width (optional) — Width in cm
     * @param height (optional) — Height in cm
     * @param fontSize (optional) — Font size in points
     * @param fontColor (optional) — Hex color
     * @param bold (optional) — Bold text
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addTextBox({ template: Struct, slideNumber?: int, text: string, x?: float, y?: float, width?: float, height?: float, fontSize?: float, fontColor?: string, bold?: bool, output: Struct }): Struct;

    /**
     * Create a blank PPTX presentation with Flow Like brand theme (16:9, Calibri, #FF4343 accent).
     * @node pptx_create @alias pptxCreate
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function create({ output: Struct }): Struct;

    /**
     * Remove a slide at the given index from a PPTX file
     * @node pptx_delete_slide @alias pptxDeleteSlide
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Index of the slide to delete (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function deleteSlide({ template: Struct, slideIndex?: int, output: Struct }): Struct;

    /**
     * Clone a slide at a given index, inserting the copy at a target position. Preserves formatting, layouts, and master references.
     * @node pptx_duplicate_slide @alias pptxDuplicateSlide
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Index of the slide to clone (1-based)
     * @param targetIndex (optional) — Position to insert the cloned slide (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function duplicateSlide({ template: Struct, slideIndex?: int, targetIndex?: int, output: Struct }): Struct;

    /**
     * Extract all text content from all slides as plain text
     * @node pptx_extract_text @alias pptxExtractText
     * @param template — Path to the PPTX file
     * @returns text — Extracted text from all slides
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Read presentation metadata (title, author, subject, keywords)
     * @node pptx_get_metadata @alias pptxGetMetadata
     * @param template — PPTX file
     * @returns title — Document title
     * @returns author — Document author
     * @returns subject — Document subject
     * @returns keywords — Document keywords
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string };

    /**
     * Scan all slides for {{...}} placeholder strings
     * @node pptx_list_placeholders @alias pptxListPlaceholders
     * @param template — Path to the PPTX file
     * @returns placeholders — List of unique placeholder names found
     * @impure has side effects / drives control flow
     */
    function listPlaceholders({ template: Struct }): string[];

    /**
     * Combine slides from multiple PPTX files into one. The base file's theme and masters are preserved.
     * @node pptx_merge @alias pptxMerge
     * @param base — Base PPTX file (theme/masters kept)
     * @param additional — Additional PPTX files to merge (array of paths)
     * @param output — Where to save the merged file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ base: Struct, additional: Struct[], output: Struct }): Struct;

    /**
     * Move a slide from one position to another
     * @node pptx_reorder_slides @alias pptxReorderSlides
     * @param template — Path to the PPTX file
     * @param fromIndex (optional) — Current position of the slide (1-based)
     * @param toIndex (optional) — Target position for the slide (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function reorderSlides({ template: Struct, fromIndex?: int, toIndex?: int, output: Struct }): Struct;

    /**
     * Replaces images in a PowerPoint (PPTX) file by matching alt text or shape name
     * @node pptx_replace_image @alias pptxReplaceImage
     * @param template — Path to the PPTX template file
     * @param identifier — Alt text or shape name of the image to replace
     * @param image — Path to the replacement image file
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Populate a table on a slide that contains a placeholder in its first cell with structured data (JSON array of arrays). Inherits the table's existing styling.
     * @node pptx_replace_table_data @alias pptxReplaceTableData
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Which slide contains the table (1-based)
     * @param placeholder — Placeholder text to find in the table
     * @param data — JSON array of arrays with table data
     * @param hasHeader (optional) — Whether the first row of data is a header row
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceTableData({ template: Struct, slideIndex?: int, placeholder: string, data: string, hasHeader?: bool, output: Struct }): Struct;

    /**
     * Replaces text placeholders in a PowerPoint (PPTX) file with plain or markdown-formatted text
     * @node pptx_replace_text @alias pptxReplaceText
     * @param template — Path to the PPTX template file
     * @param placeholder — The placeholder text to find in the template
     * @param replacement — The replacement text (supports markdown when enabled)
     * @param useMarkdown (optional) — Parse replacement text as markdown for rich formatting
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

    /**
     * Set title, author, subject, keywords in presentation metadata
     * @node pptx_set_metadata @alias pptxSetMetadata
     * @param template — PPTX file
     * @param title (optional) — Document title
     * @param author (optional) — Document author
     * @param subject (optional) — Document subject
     * @param keywords (optional) — Comma-separated keywords
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

    /**
     * Return the number of slides in a PPTX file
     * @node pptx_slide_count @alias pptxSlideCount
     * @param template — Path to the PPTX file
     * @returns count — Number of slides in the presentation
     * @impure has side effects / drives control flow
     */
    function slideCount({ template: Struct }): int;
}
