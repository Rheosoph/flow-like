Download the synthetic documents. The names and addresses are exercise data.

@PracticeFiles

| File | Expected structure |
| --- | --- |
| digital-agreement.pdf | One page with selectable text |
| scanned-agreement.pdf | One image-only page |
| mixed-agreement.pdf | Text page followed by an image-only page |
| expected.json | Facts and sensitive fields to check |

Open each PDF. Try selecting the agreement ID and email. Then upload them into a scratch app's Storage under `practice-documents/`. Use **Storage Dir → List Paths** to enumerate that prefix and **Head** to inspect file metadata without reading the entire body.

Record filename, size, page count and expected route. A file extension is only a routing hint; a renamed file may not contain the claimed format.

Keep originals intact. In a real app, choose an actual access scope before upload. A folder named `restricted` in shared Storage does not by itself restrict who can read its files.
