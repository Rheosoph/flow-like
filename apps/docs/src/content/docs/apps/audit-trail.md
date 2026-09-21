---
title: Audit trail and export
description: Read, verify and export your app's audit trail, and check an export offline without trusting the platform
---

Flow-Like records changes to your online app in a tamper-evident audit trail: who
changed what, and when. As the app's **Owner** you can read the trail, verify it,
keep a checkpoint outside the platform, and export every record to your own storage
or SIEM on any plan. An export carries everything needed to prove, without access to
Flow-Like, that no record was changed, reordered or removed.

## What is recorded

Your app has two chains:

| Chain | Holds |
| --- | --- |
| `<app-id>` | Evidence: access and membership changes, publication, content changes and deletions |
| `<app-id>#activity` | Activity: request attempts and outcomes, editor commands, graph writes, file read grants and run lifecycle. Only filled when the platform records them |

Which actions are recorded, and for how long, is set by the platform operator; see
[Platform administration](/dev/platform-administration/#audit-trail). With the
defaults, evidence stays readable in the app for about 13 months and is kept in
immutable monthly archives until the end of the third calendar year after the month,
while activity is kept for 90 days, or six months for apps assessed as high-risk AI
systems. If you must keep records longer, export them.

A record holds its time, the actor id and type (`USER`, `TECHNICAL_USER`, `API_KEY`,
`SYSTEM` or `EXECUTOR`), the action, the resource type and id, and `details` with
ids, counts and short codes. It holds the client IP only when the operator records
IPs, and only for a limited time, 7 days by default. Records contain no names or free
text; the app renders each one as a sentence from its action and resource.

## The Audit trail page

Open the app's settings and select **Audit trail**. The page lists the app's records
newest first, with a tab for the activity chain when it has records. Each record has
a status:

| Status | Meaning |
| --- | --- |
| Pending | Written, and waiting to be sealed into the chain, usually for a few minutes |
| Sealed | Part of a seal that links to the chain's previous seal |
| Invalid | Failed its integrity check before it could be sealed. Report it to the platform operator |

A record marked as redacted had its IP or details removed when their retention
ended. The record still verifies: the chain covers a salted commitment to each
value, not the value itself.

**Verify chain** checks the chain: every seal links to the one before it, every
record matches its seal, every seal is part of an epoch whose signature verifies, and
no pending record or unsigned seal was altered. The report names the first failing
seal and the problem if anything fails. Expired IPs and details are counted as
redacted, never as tampering.

A check continues after the last seal the server verified before, and says which
sequences it looked at. **Re-check from the first seal** starts again at the beginning
of what the platform still stores, which is the check to run when you want the whole
chain re-read rather than the part added since.

**Chain head** shows the chain's newest anchored seal and the signed epoch that
anchors it. Use **Copy head JSON** and store the copy outside Flow-Like. Later,
**Check a saved head** (or `POST /api/v1/audit/head/check` with
`{"seq": <epoch seq>, "hash": "<epoch hash>"}`) answers whether it matches the
current timeline, differs because the timeline was truncated or rewritten since, or
was archived because the epoch has moved to the monthly archive. Verifying the
database alone cannot notice that its newest records were removed; a saved head can.

Send the chain's seal with it — the app page does this automatically, and the API
takes `chain_id`, `seal_seq` and `seal_hash` next to the epoch — so the check also
covers your own chain. An epoch stays intact when one chain loses its newest seals,
its seal does not.

## Export the trail

Exports contain sealed records only, once a signed epoch anchors their seal, which is
normally within a few minutes of the change. A self-hosted platform without an audit
key never anchors seals, so its exports stay empty. Records carry their raw IP and
details while the platform still holds them, because you control your users' data.
The same records are delivered to a webhook and returned by the pull endpoint; both
use the [line format](#line-format) below.

### Pull

```
GET /api/v1/apps/{app_id}/audit/export?class=evidence&after_seq=0&limit=100
```

| Parameter | Meaning |
| --- | --- |
| `class` | `evidence` (default) or `activity` |
| `after_seq` | Return seals after this sequence: 0 for the start, then the previous response's `X-FlowLike-Audit-Next-Seq` |
| `limit` | Seals per page, at most 100. A page also stops at 5,000 records |

The response is NDJSON (`application/x-ndjson`). The header
`X-FlowLike-Audit-Next-Seq` holds the sequence to continue from and is absent when
there is nothing new. Authenticate as an Owner of the app, for example with a
personal access token:

```sh
after=0
while :; do
  curl -sS --fail -D headers.txt -H "Authorization: $FLOW_LIKE_PAT" \
    "https://api.example.com/api/v1/apps/$APP_ID/audit/export?class=evidence&after_seq=$after" \
    >> evidence.ndjson
  next=$(tr -d '\r' < headers.txt | awk 'tolower($1) == "x-flowlike-audit-next-seq:" { print $2 }')
  [ -z "$next" ] && break
  after=$next
done
```

Store `after` between runs to fetch only new seals.

### Webhook

Set a webhook under **Export** on the Audit trail page, or through the API:

| Request | Effect |
| --- | --- |
| `PUT /api/v1/apps/{app_id}/audit/webhook` with `{"url": "https://...", "active": true}` | Creates or replaces the webhook. Saving resets the failure count and resumes a stopped webhook. The signing secret is returned only when this call creates the webhook |
| `GET /api/v1/apps/{app_id}/audit/webhook` | URL, `active`, `failures`, `last_error`, `last_delivered_at_ms`, `next_attempt_at_ms` and the delivery cursors of both chains |
| `POST /api/v1/apps/{app_id}/audit/webhook/rotate` | Returns a new secret once; later deliveries are signed with it |
| `DELETE /api/v1/apps/{app_id}/audit/webhook` | Stops deliveries and forgets the delivery position; a new webhook starts again from the oldest seal still in the platform. The trail itself is unchanged |

The URL must use `https` and resolve to a public address; plain `http` is accepted
only by deployments that allow it for local development. The platform does not
follow redirects and waits 10 seconds for a response.

About once a minute the audit worker sends each webhook its next page as a `POST`
with an NDJSON body: the evidence chain first, then the activity chain, at most 100
seals and 5,000 records together. A backlog is delivered one page per minute. Any
`2xx` response moves the cursors past the delivered seals. Anything else is retried
after 2, 4, 8 and more minutes, at most 6 hours apart. After 50 consecutive failures
delivery stops until you switch **Deliver records** back on or save the webhook
again. `last_error` shows the HTTP status or an error class such as `timeout`, never
your response body.

Every delivery carries three headers:

| Header | Value |
| --- | --- |
| `X-FlowLike-Audit-Delivery` | A unique id for this attempt |
| `X-FlowLike-Audit-Timestamp` | Unix time in seconds when the attempt was sent |
| `X-FlowLike-Audit-Signature` | `v1=` followed by the hex HMAC-SHA256 of `<timestamp>.<body>`, keyed with the secret |

The key is the whole secret string as shown, including its `whsec_` prefix. Verify
the signature over the raw body bytes before parsing them, reject old timestamps, and
answer quickly. A page can arrive twice, for example when your `2xx` was lost, so
store records idempotently by `chain_id` and seal `seq`.

```js
import { createHmac, timingSafeEqual } from "node:crypto";

// headers: lower-cased header names, as in Node's IncomingMessage.
// rawBody: the unparsed request body as a Buffer.
export function verifyAuditDelivery(secret, headers, rawBody, toleranceSeconds = 300) {
	const timestamp = headers["x-flowlike-audit-timestamp"] ?? "";
	const signature = headers["x-flowlike-audit-signature"] ?? "";
	const age = Math.abs(Date.now() / 1000 - Number(timestamp));
	if (!/^\d+$/.test(timestamp) || age > toleranceSeconds || !signature.startsWith("v1=")) {
		return false;
	}
	const expected = createHmac("sha256", secret)
		.update(`${timestamp}.`)
		.update(rawBody)
		.digest();
	const received = Buffer.from(signature.slice(3), "hex");
	return received.length === expected.length && timingSafeEqual(received, expected);
}
```

```python
import hashlib
import hmac
import time


def verify_audit_delivery(secret: str, headers, body: bytes, tolerance: int = 300) -> bool:
    timestamp = headers.get("X-FlowLike-Audit-Timestamp", "")
    signature = headers.get("X-FlowLike-Audit-Signature", "")
    if not timestamp.isdigit() or abs(time.time() - int(timestamp)) > tolerance:
        return False
    expected = hmac.new(secret.encode(), timestamp.encode() + b"." + body, hashlib.sha256)
    return hmac.compare_digest(signature, f"v1={expected.hexdigest()}")
```

The HMAC proves that a delivery came from the platform. The records prove
themselves: verify them as described below.

## Line format

Exports, webhook bodies and the platform's monthly archives use the same format: one
JSON object per line, with a `type` of `epoch`, `seal`, `record` or `watermark`. A page
starts with the chain's watermark when records before the requested sequence were
pruned, then each epoch its seals refer to, then each seal followed by its records.
Hashes, roots and salts are lowercase hex; signatures are base64.

| `epoch` field | Meaning |
| --- | --- |
| `seq` | Position in the platform-wide timeline |
| `prev_hash` | `hash` of the previous epoch, or 64 zeros for the platform's first epoch |
| `seal_count`, `seals_root` | Number of seals the epoch commits to and the Merkle root over their hashes |
| `created_at_ms` | Creation time, Unix milliseconds |
| `kid` | Id of the audit key that signed it |
| `signature` | Raw 64-byte P-256 ECDSA signature `r ‖ s` over `hash` |
| `hash` | Epoch hash |

| `seal` field | Meaning |
| --- | --- |
| `id`, `chain_id`, `seq` | The seal and its position in the chain |
| `class` | `evidence`, or `activity` for chains ending in `#activity` |
| `prev_hash` | `hash` of the chain's previous seal, 64 zeros for the first, or the watermark's `hash` after pruning |
| `record_count`, `records_root` | Number of records and the Merkle root over their hashes |
| `first_at_ms`, `last_at_ms` | Times of the first and last record |
| `sealed_at_ms` | When the seal was written |
| `hash` | Seal hash |
| `epoch_seq`, `epoch_index`, `epoch_proof` | The epoch that anchors the seal, the seal's position in it, and the inclusion proof: sibling hashes from the leaf upward |

| `record` field | Meaning |
| --- | --- |
| `id`, `chain_id`, `seal_id` | The record, its chain and its seal. Quarantined records in archives have `seal_id` `invalid` |
| `timestamp_ms` | Unix milliseconds |
| `actor_id`, `actor_type`, `action`, `resource_type`, `resource_id` | What happened, and who did it |
| `ip_commitment`, `details_commitment` | Salted commitments the record hash covers, or `null` when the record has no such value |
| `actor_ip`, `ip_salt`, `details`, `details_salt` | The raw values and their salts while they are retained, otherwise `null`. Archives never carry the IP |

| `watermark` field | Meaning |
| --- | --- |
| `chain_id`, `seq`, `hash` | The chain's last pruned seal and its hash |
| `pruned_at_ms` | When it was pruned |
| `batch_root`, `batch_size`, `batch_index`, `batch_proof` | One signature covers every watermark of a prune run: the Merkle root over their hashes, how many there are, this watermark's position and its inclusion proof |
| `kid`, `signature` | Audit key id and signature over the batch hash |

## Verify offline

Offline verification needs the NDJSON lines and the public key for each `kid` that
appears in them. Ask the platform operator for those keys; they are P-256 SPKI PEM
public keys.

**Canonical JSON.** Every hash is BLAKE3 (32 bytes) of the UTF-8 bytes of a canonical
JSON object. Object keys are sorted by their UTF-8 bytes, and nothing is written
between tokens. Strings escape `"` and `\` with a backslash, use `\b`, `\f`, `\n`,
`\r` and `\t`, write other control characters as `\u00xx` in lowercase hex, and write
everything else, including non-ASCII characters and `/`, as is. `true`, `false` and
`null` are written as is. A number is written as its significant digits, then `e`,
then the decimal exponent, with no leading or trailing zeros in the digits: `1e0`
for 1, `17582832e5` for 1758283200000, `15e-1` for 1.5 and `-2e0` for -2. Zero is
`0`.

**Hashes.** Each hash covers an object made of a `domain` and the fields listed,
taken from the line:

| Hash | `domain` | Fields |
| --- | --- | --- |
| Record | `flow-like.audit-record/v1` | `id`, `chain_id`, `timestamp_ms`, `actor_id`, `actor_type`, `action`, `resource_type`, `resource_id`, `ip_commitment`, `details_commitment` |
| Seal | `flow-like.audit-seal/v1` | `id`, `chain_id`, `seq`, `class`, `prev_hash`, `record_count`, `records_root`, `first_at_ms`, `last_at_ms`, `sealed_at_ms` |
| Epoch | `flow-like.audit-epoch/v1` | `seq`, `prev_hash`, `seal_count`, `seals_root`, `created_at_ms`, `kid` |
| Watermark | `flow-like.audit-watermark/v1` | `chain_id`, `seq`, `hash`, `pruned_at_ms` |
| Watermark batch | `flow-like.audit-watermark-batch/v1` | `root` (the line's `batch_root`), `size` (`batch_size`), `pruned_at_ms`, `kid` |
| Archive manifest | `flow-like.audit-manifest/v1` | `manifest`: the whole manifest without its `signature` field |

**Commitments.** The IP commitment is BLAKE3 in keyed mode, with the 32-byte salt as
the key, over the bytes `flow-like.audit-ip/v1`, a zero byte and the IP. The details
commitment is the same over `flow-like.audit-details/v1`, a zero byte and the
canonical JSON of `details`.

**Merkle trees** follow RFC 9162 with BLAKE3: a leaf is BLAKE3 of a `0x00` byte and
the 32-byte hash, an inner node is BLAKE3 of a `0x01` byte, the left and the right
child, a tree over `n > 1` leaves splits at the largest power of two below `n`, and the
empty tree is BLAKE3 of nothing. A seal's leaves are its records' hashes ordered by
`timestamp_ms`, then by `id` bytes. An epoch's leaves are its seals' hashes in
`epoch_index` order, and a watermark batch's leaves are its watermarks' hashes in
`batch_index` order. Inclusion proofs are checked as in RFC 9162 section 2.1.3.2.

**Signatures** are standard ECDSA P-256 with SHA-256 (ES256) whose message is the
32 raw bytes of the hash. Any ECDSA library verifies them; convert `r ‖ s` to DER if
the library expects it.

The checks, in order:

1. Each epoch's hash matches its fields and its signature verifies. Consecutive
   epochs link: `prev_hash` equals the previous epoch's `hash`.
2. Each watermark's hash leads through `batch_proof` to `batch_root`, and the
   signature over the batch hash verifies.
3. For each record whose raw values are present, the commitments match. Absent
   values with a commitment are redacted, not tampered with.
4. For each seal, the records with its `seal_id` are complete (`record_count`), their
   hashes produce `records_root`, their times match `first_at_ms` and `last_at_ms`,
   `class` matches the chain, and the seal's hash matches its fields.
5. Each seal's inclusion proof leads from its hash to its epoch's `seals_root`.
6. Within a chain, seal `seq` values are consecutive and each `prev_hash` equals the
   previous seal's `hash`. The first seal links to 64 zeros, or to the watermark's
   `hash` when the chain was pruned before it.

Pages verify together: pass all pages of a chain in order, and seals link across
them. A retained head adds the last check: its epoch must still be in the timeline,
or in the monthly archive once pruned.

This script implements all of the checks:

```python
"""Verify Flow-Like audit NDJSON offline: export pages, webhook bodies or archive parts.

pip install blake3 cryptography zstandard
python verify_audit.py --key <kid>=<public-key.pem> page-1.ndjson page-2.ndjson
python verify_audit.py --key <kid>=<pem> --after <chain>=<seq>:<hash> 2026/09/0001.jsonl.zst
"""

import argparse
import base64
import io
import json
import sys
from decimal import Decimal

import blake3
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature

ZERO = "00" * 32
RECORD_FIELDS = ("id", "chain_id", "timestamp_ms", "actor_id", "actor_type", "action",
                 "resource_type", "resource_id", "ip_commitment", "details_commitment")
SEAL_FIELDS = ("id", "chain_id", "seq", "class", "prev_hash", "record_count", "records_root",
               "first_at_ms", "last_at_ms", "sealed_at_ms")
EPOCH_FIELDS = ("seq", "prev_hash", "seal_count", "seals_root", "created_at_ms", "kid")
WATERMARK_FIELDS = ("chain_id", "seq", "hash", "pruned_at_ms")
BATCH_FIELDS = ("root", "size", "pruned_at_ms", "kid")


class VerificationError(Exception):
    pass


def require(condition: bool, problem: str):
    if not condition:
        raise VerificationError(problem)


def canonical(value) -> str:
    """Sorted keys, no whitespace, numbers as <digits>e<exponent>."""
    if isinstance(value, dict):
        items = (f"{json.dumps(key, ensure_ascii=False)}:{canonical(value[key])}"
                 for key in sorted(value))
        return "{" + ",".join(items) + "}"
    if isinstance(value, list):
        return "[" + ",".join(canonical(item) for item in value) + "]"
    if isinstance(value, bool) or value is None:
        return json.dumps(value)
    if isinstance(value, (int, float, Decimal)):
        number = Decimal(repr(value)) if isinstance(value, float) else Decimal(value)
        sign, digits, exponent = number.as_tuple()
        text = "".join(map(str, digits)).lstrip("0")
        if not text:
            return "0"
        trimmed = text.rstrip("0")
        return f"{'-' if sign else ''}{trimmed}e{exponent + len(text) - len(trimmed)}"
    return json.dumps(value, ensure_ascii=False)


def digest(domain: str, line: dict, fields: tuple) -> bytes:
    document = {"domain": domain, **{field: line[field] for field in fields}}
    return blake3.blake3(canonical(document).encode()).digest()


def commitment(salt_hex: str, context: bytes, data: bytes) -> str:
    return blake3.blake3(context + data, key=bytes.fromhex(salt_hex)).hexdigest()


def leaf(data: bytes) -> bytes:
    return blake3.blake3(b"\x00" + data).digest()


def node(left: bytes, right: bytes) -> bytes:
    return blake3.blake3(b"\x01" + left + right).digest()


def merkle_root(leaves: list) -> bytes:
    if not leaves:
        return blake3.blake3(b"").digest()
    if len(leaves) == 1:
        return leaf(leaves[0])
    split = 1
    while split * 2 < len(leaves):
        split *= 2
    return node(merkle_root(leaves[:split]), merkle_root(leaves[split:]))


def included(data: bytes, index: int, size: int, proof: list, root: bytes) -> bool:
    """RFC 9162 section 2.1.3.2."""
    if index >= size:
        return False
    fn, sn, result = index, size - 1, leaf(data)
    for sibling in proof:
        if sn == 0:
            return False
        if fn & 1 or fn == sn:
            result = node(sibling, result)
            while not fn & 1 and fn != 0:
                fn, sn = fn >> 1, sn >> 1
        else:
            result = node(result, sibling)
        fn, sn = fn >> 1, sn >> 1
    return sn == 0 and result == root


def check_signature(keys: dict, kid: str, message: bytes, signature_b64: str, what: str):
    require(kid in keys, f"{what}: no public key for kid {kid}; pass --key {kid}=<pem>")
    raw = base64.b64decode(signature_b64)
    r, s = int.from_bytes(raw[:32], "big"), int.from_bytes(raw[32:], "big")
    try:
        keys[kid].verify(encode_dss_signature(r, s), message, ec.ECDSA(hashes.SHA256()))
    except InvalidSignature:
        raise VerificationError(f"{what}: signature does not verify") from None


def check_raw(record: dict, value: str, salt: str, commitment_field: str, context: bytes, data) -> None:
    """Value, salt and matching commitment, or neither value nor salt: redacted when the
    commitment remains, never recorded when it is absent too."""
    name = f"record {record['id']} {value}"
    if record[value] is None and record[salt] is None:
        return
    require(None not in (record[value], record[salt], record[commitment_field]),
            f"{name}: value, salt and commitment must be present together")
    try:
        key = bytes.fromhex(record[salt])
    except ValueError:
        key = b""
    require(len(key) == 32, f"{name}: salt is not 32 bytes of hex")
    expected = blake3.blake3(context + data(record[value]), key=key).hexdigest()
    require(expected == record[commitment_field], f"{name} does not match its commitment")


def record_hash(record: dict) -> bytes:
    check_raw(record, "actor_ip", "ip_salt", "ip_commitment", b"flow-like.audit-ip/v1\0", str.encode)
    check_raw(record, "details", "details_salt", "details_commitment", b"flow-like.audit-details/v1\0",
              lambda details: canonical(details).encode())
    return digest("flow-like.audit-record/v1", record, RECORD_FIELDS)


def check_seal(seal: dict, records: list, epochs: dict, keyless: bool) -> None:
    name = f"{seal['chain_id']} seal {seal['seq']}"
    records.sort(key=lambda record: (record["timestamp_ms"], record["id"].encode()))
    require(records and len(records) == seal["record_count"],
            f"{name}: {len(records)} of {seal['record_count']} records present")
    root = merkle_root([record_hash(record) for record in records])
    require(root.hex() == seal["records_root"], f"{name}: records do not match the root")
    require(records[0]["timestamp_ms"] == seal["first_at_ms"]
            and records[-1]["timestamp_ms"] == seal["last_at_ms"],
            f"{name}: record times fall outside the seal")
    expected_class = "activity" if seal["chain_id"].endswith("#activity") else "evidence"
    require(seal["class"] == expected_class, f"{name}: wrong class")
    hash_ = digest("flow-like.audit-seal/v1", seal, SEAL_FIELDS)
    require(hash_.hex() == seal["hash"], f"{name}: hash does not match its fields")
    if seal["epoch_seq"] is None:
        require(keyless, f"{name}: no epoch anchors it; only archives of a deployment "
                         "without an audit key hold such seals (--keyless-archive)")
        return
    epoch = epochs.get(seal["epoch_seq"])
    require(epoch is not None, f"{name}: epoch {seal['epoch_seq']} is missing")
    proof = [bytes.fromhex(sibling) for sibling in seal["epoch_proof"]]
    root = bytes.fromhex(epoch["seals_root"])
    require(included(hash_, seal["epoch_index"], epoch["seal_count"], proof, root),
            f"{name}: not part of epoch {seal['epoch_seq']}")


def read_lines(path: str):
    if path.endswith(".zst"):
        import zstandard

        with open(path, "rb") as compressed:
            reader = zstandard.ZstdDecompressor().stream_reader(compressed)
            yield from (line for line in io.TextIOWrapper(reader, encoding="utf-8") if line.strip())
    else:
        with open(path, encoding="utf-8") as plain:
            yield from (line for line in plain if line.strip())


def add_anchor(anchors: dict, chain_id: str, seq: int, hash_: str) -> None:
    known = anchors.setdefault(chain_id, {}).setdefault(seq, hash_)
    require(known == hash_, f"{chain_id}: two different hashes for seal {seq}")


def check_links(chain_id: str, chain: list, anchors: dict) -> None:
    """Seals follow each other from seq 1. A gap, including before the first seal, is
    allowed only where a signed watermark (or --after) names the seal before it."""
    chain.sort(key=lambda seal: seal["seq"])
    previous = (0, ZERO)
    for seal in chain:
        seq = seal["seq"]
        require(seq != previous[0], f"{chain_id} seal {seq} appears twice; pass each page once")
        if seq != previous[0] + 1:
            bridge = anchors.get(seq - 1)
            require(bridge is not None, f"{chain_id}: seals {previous[0] + 1} to {seq - 1} are missing "
                                        "and no watermark covers them")
            previous = (seq - 1, bridge)
        require(seal["prev_hash"] == previous[1], f"{chain_id} seal {seq}: does not link to seal {previous[0]}")
        require(anchors.get(seq, seal["hash"]) == seal["hash"], f"{chain_id} seal {seq}: differs from its watermark")
        previous = (seq, seal["hash"])


def verify(paths: list, keys: dict, keyless: bool = False, anchors: dict | None = None) -> dict:
    epochs, seals, members, anchors = {}, [], {}, anchors or {}
    for path in paths:
        for text in read_lines(path):
            line = json.loads(text, parse_float=Decimal)
            kind = line.pop("type")
            if kind == "epoch":
                hash_ = digest("flow-like.audit-epoch/v1", line, EPOCH_FIELDS)
                require(hash_.hex() == line["hash"], f"epoch {line['seq']}: hash does not match")
                check_signature(keys, line["kid"], hash_, line["signature"], f"epoch {line['seq']}")
                known = epochs.setdefault(line["seq"], line)
                require(known["hash"] == line["hash"], f"epoch {line['seq']}: two different epochs")
            elif kind == "seal":
                seals.append(line)
            elif kind == "record" and line["seal_id"] != "invalid":
                members.setdefault(line["seal_id"], []).append(line)
            elif kind == "watermark":
                name = f"watermark of {line['chain_id']}"
                hash_ = digest("flow-like.audit-watermark/v1", line, WATERMARK_FIELDS)
                proof = [bytes.fromhex(sibling) for sibling in line["batch_proof"]]
                require(included(hash_, line["batch_index"], line["batch_size"], proof,
                                 bytes.fromhex(line["batch_root"])),
                        f"{name}: not part of its batch")
                batch = {"root": line["batch_root"], "size": line["batch_size"],
                         "pruned_at_ms": line["pruned_at_ms"], "kid": line["kid"]}
                batch_hash = digest("flow-like.audit-watermark-batch/v1", batch, BATCH_FIELDS)
                check_signature(keys, line["kid"], batch_hash, line["signature"], name)
                add_anchor(anchors, line["chain_id"], line["seq"], line["hash"])

    for seq, epoch in epochs.items():
        previous = epochs.get(seq - 1)
        require(previous is None or epoch["prev_hash"] == previous["hash"], f"epoch {seq}: broken link")

    chains = {}
    for seal in seals:
        check_seal(seal, members.pop(seal["id"], []), epochs, keyless)
        chains.setdefault(seal["chain_id"], []).append(seal)
    require(not members, f"records without their seal: {sorted(members)}")

    for chain_id, chain in chains.items():
        check_links(chain_id, chain, anchors.get(chain_id, {}))
    return chains


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--key", action="append", default=[], help="<kid>=<SPKI PEM file>")
    parser.add_argument("--after", action="append", default=[],
                        help="<chain>=<seq>:<hash> of a seal verified earlier; the chain continues after it")
    parser.add_argument("--keyless-archive", action="store_true",
                        help="accept seals without an epoch, as archived by a deployment without an audit key")
    parser.add_argument("files", nargs="+")
    args = parser.parse_args()
    keys, anchors = {}, {}
    for entry in args.key:
        kid, _, path = entry.partition("=")
        with open(path, "rb") as pem:
            keys[kid] = serialization.load_pem_public_key(pem.read())
    for entry in args.after:
        chain_id, _, point = entry.partition("=")
        seq, _, hash_ = point.partition(":")
        add_anchor(anchors, chain_id, int(seq), hash_.lower())
    for chain_id, chain in verify(args.files, keys, args.keyless_archive, anchors).items():
        anchored = sum(seal["epoch_seq"] is not None for seal in chain)
        last = chain[-1]
        print(f"{chain_id}: seals {chain[0]['seq']}..{last['seq']} verified, {anchored} anchored; "
              f"continue with --after '{chain_id}={last['seq']}:{last['hash']}'")


if __name__ == "__main__":
    try:
        main()
    except VerificationError as error:
        sys.exit(f"verification failed: {error}")
```

The platform's monthly archives use the same lines, compressed with zstd, in parts
listed by a signed `manifest.json`. The script reads `.zst` parts directly. Check each
part's SHA-256 against the manifest, and the manifest's signature over its hash
without the `signature` field. Archives hold evidence only, never raw IPs, and no
watermarks: a month's seals continue the chain of the month before.
