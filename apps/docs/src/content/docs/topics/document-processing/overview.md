---
title: Document Processing
description: Extract, transform, and process PDFs, spreadsheets, and documents at scale
sidebar:
  order: 1
---

Flow-Like provides powerful document processing capabilities—extract text from PDFs, process Excel files, batch-transform documents, and use AI for intelligent extraction.

## Supported Document Types

| Format | Capabilities |
|--------|--------------|
| **PDF** | Page count, render to images, text extraction |
| **Excel (.xlsx)** | Read/write cells, manage worksheets, extract tables |
| **CSV** | Stream reading, database conversion, SQL queries |
| **Images** | OCR, resize, crop, rotate, convert formats |
| **Word (.docx)** | Text extraction |
| **HTML** | Convert to Markdown, extract content |

## PDF Processing

### Get Page Count

```
PDF Page Count (file_path)
    │
    ▼
Number: 42
```

### Render Pages as Images

Process each page visually:

```
PDF To Images (file_path)
    │
    ▼
Array<Image>: [page1.png, page2.png, ...]
```

Or render a specific page:

```
PDF Page To Image
├── file: document.pdf
├── page: 1 (1-based)
└── scale: 2.0 (for high resolution)
    │
    ▼
Image (PNG)
```

### Extract Text with AI

For complex PDFs with mixed layouts:

```
AI Extract Document
├── file: complex_document.pdf
├── model: GPT-4 Vision
└── extract_images: true
    │
    ▼
Markdown text with structure preserved
```

**What AI extraction handles:**
- Multi-column layouts
- Tables and charts
- Handwritten text
- Mixed text and images
- Scanned documents

### Example: Invoice Processing Pipeline

```
Quick Action Event (pdf_files: Array<Path>)
    │
    ▼
For Each pdf_file
    │
    ▼
AI Extract Document
    │
    ▼
Extract Knowledge (Invoice Schema)
├── vendor: String
├── invoice_number: String
├── date: Date
├── line_items: Array<{description, quantity, price}>
└── total: Number
    │
    ▼
Insert to Database ──▶ Return summary
```

## Excel Processing

### Read/Write Cells

```
Excel Read Cell
├── file: report.xlsx
├── sheet: "Sales"
└── cell: "B5"
    │
    ▼
Value: 45230.00
```

```
Excel Write Cell
├── file: report.xlsx
├── sheet: "Sales"
├── cell: "C5"
└── value: "Processed"
```

### Manage Worksheets

```
Get Sheet Names (file)
    │
    ▼
["Sales", "Inventory", "Summary"]
```

```
New Worksheet
├── file: report.xlsx
└── name: "Q4 Results"
```

```
Copy Worksheet
├── source: template.xlsx
├── source_sheet: "Template"
├── target: report.xlsx
└── target_sheet: "January"
```

### Loop Through Rows

Process all rows in a worksheet:

```
For Each Row (file: data.xlsx, sheet: "Customers")
    │
    ├── row.A ──▶ customer_id
    ├── row.B ──▶ name
    └── row.C ──▶ email
    │
    ▼
Process each customer
```

### Extract Tables Intelligently

For messy Excel files with multiple tables:

```
AI Extract Tables
├── file: messy_report.xlsx
├── model: GPT-4
└── Strategy determined by AI
    │
    ▼
Array of structured tables
```

The AI:
1. Analyzes the spreadsheet structure
2. Identifies table boundaries
3. Determines headers
4. Extracts clean data

### Microsoft 365 Excel

Work with Excel files in OneDrive/SharePoint:

```
Microsoft Provider (OAuth)
    │
    ▼
List Excel Worksheets
├── file_id: "abc123"
└── site_id (optional for SharePoint)
    │
    ▼
Read Excel Range
├── sheet: "Data"
└── range: "A1:D100"
    │
    ▼
Array of rows
```

## CSV Processing

### Stream Large Files

Process CSV files without loading everything into memory:

```
Buffered CSV Reader (large_file.csv)
    │
    ▼
For Each batch (1000 rows)
    │
    ▼
Process batch ──▶ Insert to database
```

### Convert to Database

Load CSV into queryable format:

```
Create Database (lance_db)
    │
    ▼
Load CSV (sales.csv)
    │
    ▼
SQL Query: "SELECT * FROM sales WHERE amount > 1000"
```

### DataFusion Integration

Query CSV files with SQL:

```
Create DataFusion Session
    │
    ▼
Register CSV ("sales", sales.csv)
    │
    ▼
Register CSV ("customers", customers.csv)
    │
    ▼
SQL Query:
"SELECT c.name, SUM(s.amount) as total
 FROM sales s
 JOIN customers c ON s.customer_id = c.id
 GROUP BY c.name
 ORDER BY total DESC"
```

## Image Processing

### Read & Analyze

```
Read Image (photo.jpg)
    │
    ├── Image Dimensions ──▶ {width: 1920, height: 1080}
    │
    └── AI Extract Document ──▶ Extracted text/content
```

### Transform Images

| Node | Description |
|------|-------------|
| **Resize** | Scale to specific dimensions |
| **Crop** | Extract region |
| **Rotate** | Rotate by degrees |
| **Flip** | Horizontal or vertical flip |
| **Blur** | Apply blur effect |
| **Brighten** | Adjust brightness |
| **Contrast** | Adjust contrast |
| **Convert** | Change format (PNG, JPG, WebP) |

**Example: Prepare images for processing**
```
Read Image
    │
    ▼
Resize (max_width: 1024)
    │
    ▼
Convert to PNG
    │
    ▼
AI Analysis
```

### Barcode & QR Reading

```
Read Barcodes (image)
    │
    ▼
[{
  type: "QR_CODE",
  data: "https://example.com/product/123",
  bounds: {x, y, width, height}
}]
```

### Draw Annotations

Add bounding boxes or annotations:

```
Draw Boxes
├── image: document.png
├── boxes: [{x, y, w, h, label: "Invoice Number"}]
└── color: red
    │
    ▼
Annotated image
```

### Generate Barcodes

```
Write Barcode
├── data: "https://myapp.com/verify/abc123"
├── format: QR_CODE
└── scale: 8
    │
    ▼
Barcode image
```

## Text Extraction

### HTML to Markdown

Clean up web content:

```
HTML to Markdown
├── html: "<h1>Title</h1><p>Content...</p>"
└── remove_tags: ["script", "style", "nav"]
    │
    ▼
"# Title\n\nContent..."
```

### Keyword Extraction

**YAKE (Unsupervised):**
```
YAKE Keywords
├── text: document_content
├── language: "en"
└── max_keywords: 10
    │
    ▼
["machine learning", "data processing", "automation", ...]
```

**RAKE (Rule-based):**
```
RAKE Keywords
├── text: document_content
└── language: "en"
    │
    ▼
[{keyword: "artificial intelligence", score: 8.5}, ...]
```

**AI-Powered:**
```
AI Keyword Extraction
├── text: document_content
└── model: GPT-4
    │
    ▼
Semantically relevant keywords
```

## Batch Processing

### Process Folder of Documents

```
Quick Action Event (folder_path)
    │
    ▼
List Paths (folder_path, pattern: "*.pdf")
    │
    ▼
For Each file_path
    │
    ▼
Detect file type
    │
    ├── PDF ──▶ AI Extract Document
    ├── Excel ──▶ Extract Tables
    ├── Image ──▶ OCR Extract
    └── CSV ──▶ Load to Database
    │
    ▼
Store extracted data ──▶ Generate report
```

### Watch Folder for New Files

```
Scheduled Event (every 5 minutes)
    │
    ▼
List Paths (/incoming, modified_after: last_run)
    │
    ▼
For Each new_file
    │
    ▼
Process document ──▶ Move to /processed
```

## AI-Powered Processing

### Structured Extraction

Extract specific fields from any document:

```
AI Extract Document (document)
    │
    ▼
Extract Knowledge
├── Schema:
│   ├── company_name: String
│   ├── document_type: Enum["invoice", "receipt", "contract"]
│   ├── date: Date
│   ├── total_amount: Number
│   └── line_items: Array<{description, amount}>
│
└── Model: GPT-4
    │
    ▼
Validated structured data
```

### Document Classification

```
AI Classification
├── document: extracted_text
├── categories: ["Invoice", "Receipt", "Contract", "Report", "Letter"]
└── model: GPT-4
    │
    ▼
{
  category: "Invoice",
  confidence: 0.95
}
```

### Summarization

```
Invoke LLM
├── prompt: "Summarize this document in 3 bullet points: {document_text}"
└── model: GPT-4
    │
    ▼
• Key point 1
• Key point 2
• Key point 3
```

## Template Processing

Generate documents from templates:

```
Render Template
├── template: "Dear {name},\n\nYour order #{order_id} has shipped..."
├── variables:
│   ├── name: "Alice"
│   └── order_id: "12345"
    │
    ▼
"Dear Alice,\n\nYour order #12345 has shipped..."
```

**Jinja-style features:**
- Variable interpolation: `{variable}`
- Conditionals: `{% if condition %}...{% endif %}`
- Loops: `{% for item in items %}...{% endfor %}`
- Filters: `{name|upper}`

## File Operations

### Basic Operations

| Node | Description |
|------|-------------|
| **Copy** | Copy file to new location |
| **Delete** | Remove file |
| **Rename** | Rename/move file |
| **Exists** | Check if file exists |
| **File Hash** | Compute MD5/SHA hash |

### Cloud Storage

Work with files in cloud storage:

```
S3 / Azure / GCS Provider
    │
    ▼
List Files (bucket/container)
    │
    ▼
Download File
    │
    ▼
Process locally
    │
    ▼
Upload results
```

### Signed URLs

Generate temporary access URLs:

```
Sign URL
├── path: "reports/quarterly.pdf"
├── expiry: 3600 (seconds)
└── provider: S3
    │
    ▼
"https://bucket.s3.amazonaws.com/reports/quarterly.pdf?signature=..."
```

## Example Pipelines

### Invoice Processing

```
Watch Folder (/invoices)
    │
    ▼
For Each new PDF
    │
    ├──▶ AI Extract Document
    │
    ├──▶ Extract Knowledge (Invoice Schema)
    │
    ├──▶ Validate required fields
    │       │
    │       ├── Valid ──▶ Insert to Database
    │       │               │
    │       │               ▼
    │       │           Create Approval Task
    │       │
    │       └── Invalid ──▶ Move to /review
    │
    └──▶ Move to /processed
```

### Document Search System

```
Ingest Pipeline:
├── List all documents
├── For Each document
│   ├── Extract text (AI Extract Document)
│   ├── Chunk into sections
│   ├── Generate embeddings
│   └── Insert to Vector DB
│
Query Pipeline:
├── User search query
├── Embed query
├── Vector search (top 10)
├── Return matching documents with snippets
```

### Report Generation

```
Scheduled Event (monthly)
    │
    ▼
Query Database (monthly stats)
    │
    ▼
Generate Charts (Nivo)
    │
    ▼
Render Template (report_template.md)
    │
    ▼
Convert to PDF
    │
    ▼
Email Report ──▶ Archive
```

## Best Practices

### 1. Handle Encoding
Always specify encoding for text files:
```
Read to String (file, encoding: "utf-8")
```

### 2. Validate Before Processing
Check file type and size before heavy processing:
```
File Exists → Get File Size → Validate → Process
```

### 3. Use Appropriate Extraction
| Document Type | Best Approach |
|---------------|---------------|
| Clean PDF | Direct text extraction |
| Scanned PDF | AI vision OCR |
| Structured Excel | Cell/range reading |
| Messy Excel | AI table extraction |
| Mixed content | AI Extract Document |

### 4. Batch Wisely
For large volumes, process in batches to manage memory:
```
For Each batch of 100 files
    │
    ▼
Process batch ──▶ Save results ──▶ Next batch
```

### 5. Archive Originals
Keep original documents before processing:
```
Copy to /archive ──▶ Process ──▶ Store results
```

## Next Steps

- **[Data Loading](/topics/datascience/loading/)** – Store extracted data
- **[DataFusion](/topics/datascience/datafusion/)** – Query processed data
- **[GenAI](/topics/genai/extraction/)** – Advanced AI extraction
- **[Building Internal Tools](/topics/internal-tools/overview/)** – Create document processing UIs
