A support agent uploads a customer's contract to "have a colleague take a quick look" — and it lands where the entire team can browse it. Nobody hacked anything; someone just picked the wrong storage scope. In Flow-Like, *where* a file lives is an access decision, which makes it the first decision, not an afterthought.

## 1 · Pick the scope

Open the copilot's **Storage** workspace from the sidebar's Data section.

@AppStorage

That's the copilot's shared Storage: two folders (`archived-tickets`, `customer-briefs`) and three files — `brand-voice.md`, `refund-policy.md`, and the 2.8 MB `support-playbook.pdf` — with **New Folder**, **Upload Files**, and **Upload Folder** buttons along the top. Everything here is team-visible and flow-readable, which is exactly right for policies and playbooks. **User Storage**, one entry below in the sidebar, is the private mirror: personal uploads and drafts other app users can't see.

Rules of thumb: shared source data and approved outputs go to Storage; personal drafts and private uploads go to User Storage; secrets go to neither — a user-scoped path is not a vault. And derived files inherit their source's sensitivity: an index built from a restricted document is itself restricted.

## 2 · Treat paths as typed values

Flow-Like nodes pass a typed path that points into a storage scope, not a raw OS path. Directory nodes hand you roots (app, user, cache, upload); path nodes append children, swap extensions, inspect parents. Build child paths from a known root and validate anything user-supplied: reject empty names, `..` traversal, and unexpected separators. This abstraction is what keeps the same flow working across local and hosted deployments.

Signed URLs deserve the same respect as passwords: they're temporary capabilities. Limit the method, keep the lifetime short, don't log the query string, and check the caller's access *before* signing — expiry is not authorization.

## 3 · Ingest the archived tickets

The `archived-tickets` folder holds CSV exports from the team's old helpdesk. Next lesson you'll import them into a table; the file half of that pipeline is this lesson's job, and it has to survive being run twice.

1. Take the uploaded path from the event, or pick the file from Storage.
2. Check extension and size before reading the whole body — an extension is a claim, not proof.
3. Compute a content hash and pair it with a stable source ID. Record both in a small manifest table: source ID, original name, hash, size, time, status.
4. Parse into typed rows, collecting validation errors with row locations.
5. Write accepted rows to a native table — and only then move the file into a processed archive location.
6. On retry, look up the hash first: same version means skip or replace, never duplicate.

The order matters. Move-then-parse means a failed parse leaves the file archived with zero rows written — a state that looks finished and isn't. The manifest is what makes the whole lifecycle observable: you can always answer "which version of which file produced these rows?"

**Watch out:** a filename is a label, not an identity. `tickets-export.csv` uploaded twice may be two different versions; the hash plus source ID tells them apart, the name never will.

## Recap

- Scope is access: Storage is team-visible, User Storage is private, and derived files inherit their source's sensitivity.
- Use typed paths from known roots, validate user-supplied names, and treat signed URLs like credentials.
- Ingest with hash + source ID in a manifest, and archive only after rows are safely written.
