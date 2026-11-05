# flow-bits

A command line tool for bits, covering both halves of the system: the local
store the desktop app downloads into, and the bit catalog a hub serves.

```bash
cargo run -p bit-cli -- <command>     # or: cargo bits <command>
```

Global options: `--store <DIR>` for the local store, `--hub <DOMAIN>` for the
catalog, `--token <PAT>` for catalog writes. Without them the tool reads the
desktop app's `global-settings.json`, falling back to the platform defaults and
`api.flow-like.com`.

## Local store

| Command | What it does |
| --- | --- |
| `status` | Store path, artifact count, size, problem count |
| `list [--resolve] [--json]` | Artifacts on disk, named against the catalog with `--resolve` |
| `search <query> [--type Llm]` | Search the catalog, marking what is installed |
| `info <bit-id>` | A bit, its dependency pack and which artifacts are present |
| `install <bit-id>…` | Download a bit and its dependencies |
| `remove <bit-id\|hash>… [--yes]` | Delete store directories, printing the plan without `--yes` |
| `doctor [--fix]` | Empty directories, superseded partials and zero byte artifacts |

Downloads run through the runtime's own path, so resume, size validation and
blake3 verification behave exactly as they do in the desktop app.

## Hub catalog

| Command | What it does |
| --- | --- |
| `hub whoami` | The user the token acts as, and whether it may write bits |
| `hub pull <bit-id> [--with-dependencies] [--out FILE]` | Turn catalog entries into a spec |
| `hub push <spec> [--dry-run]` | Write a spec: dependencies first, then metadata |
| `hub delete <bit-id>… [--yes]` | Remove entries from the catalog |

Writes need an Admin-level personal access token whose user holds `WriteBits`
(or `Admin`), passed as `--token` or `FLOW_LIKE_PAT`. Mint one under settings,
or through `PUT /user/pat`.

### Spec format

One file describes a bit and everything it depends on. An entry carries the
fields of a `Bit` plus two authoring conveniences: `ref`, a local alias other
entries point at as `@alias`, and `meta`, the per-language metadata the hub
stores through its own endpoint. Omit `id` and one is minted.

```json
{
  "bits": [
    {
      "ref": "tokenizer",
      "type": "Tokenizer",
      "download_link": "https://huggingface.co/org/model/resolve/main/tokenizer.json",
      "file_name": "tokenizer.json",
      "license": "apache-2.0",
      "meta": { "en": { "name": "Tokenizer", "description": "…" } }
    },
    {
      "ref": "model",
      "type": "Llm",
      "download_link": "https://huggingface.co/org/model/resolve/main/model.gguf",
      "file_name": "model.gguf",
      "dependencies": ["@tokenizer"],
      "parameters": { "context_length": 131072, "provider": { "provider_name": "Local" } },
      "meta": { "en": { "name": "The model", "description": "…" } }
    }
  ]
}
```

A hosted bit is the same shape without `download_link`, naming its profile
under `parameters.provider` (for example `hosted:openrouter` with a
`@preset/…` model id). `hub pull` on an existing entry is the quickest way to
get a correct starting point for either kind.

The hub fetches, hashes and mirrors every artifact itself, so a spec only ever
names source URLs. Dependencies are stored as `hub:id`; the tool writes each
dependency first and builds the reference from what the hub stored.

Forecasting entries also accept aliases in `parameters.assets[].bit`:

```json
{
  "model_type": "chronos2",
  "provider": { "provider_name": "local:forecast" },
  "assets": [
    { "bit": "@config", "relative_path": "config.json" },
    { "bit": "@shard", "relative_path": "model-00001-of-00002.safetensors" },
    { "bit": "models.example:existing-shard", "relative_path": "model-00002-of-00002.safetensors" }
  ]
}
```

Use `type: "Forecasting"` on the model entry and publish its config and shards
as separate `Config` or `File` entries. Asset references automatically become
dependencies, so they do not need to be repeated in `dependencies`. The tool
resolves aliases using the IDs returned by the Hub, including when the Hub
reuses an existing artifact. Qualified remote references stay unchanged;
bare IDs use the target Hub.

The model entry's own `file_name` supplies `model.safetensors` or
`model.safetensors.index.json`. Omit that file from `assets`; a self-reference
using the same filename is accepted and removed as redundant. Other asset
filenames must refer to a separate Bit. `hub pull --with-dependencies`
preserves local asset aliases for later publication.

### Publish the forecasting catalog

The checked-in bundle contains the supported forecasting checkpoints and their
config and license assets, with source revisions pinned. From the repository
root, inspect its dependency plan before publishing to a Hub that supports
Forecasting Bits:

```bash
cargo run -p bit-cli -- --hub api.flow-like.com hub push apps/utils/bit-cli/specs/forecasting.json --dry-run
cargo run -p bit-cli -- --hub api.flow-like.com hub push apps/utils/bit-cli/specs/forecasting.json
```

The second command uses the catalog-write credentials described above and
publishes the artifacts. Review each checkpoint's license before using it;
the Timestral entries include a revenue restriction in their metadata and
license files.

### Publish the OpenRouter catalog

`specs/hosted-openrouter.json` contains 127 hosted Bits: 31 text-to-speech,
27 speech-to-text, 31 image generation, and 38 embedding models.
The target Hub needs the corresponding hosted API routes and a
server-side `OPENROUTER_API_KEY`. The bundle contains
model identifiers and metadata, with no credentials or model downloads.

```bash
cargo run -p bit-cli -- --hub api.flow-like.com hub push apps/utils/bit-cli/specs/hosted-openrouter.json --dry-run
cargo run -p bit-cli -- --hub api.flow-like.com hub push apps/utils/bit-cli/specs/hosted-openrouter.json
```

Keep the bundle's IDs when updating its entries. A Hub can return its own stored
ID on the first push. For hosted entries without files, the CLI uses that ID to
apply parameter updates when the Hub reuses an existing entry. The printed IDs
are the catalog IDs to select in a profile or pin to a bot.

Model IDs, default voices, embedding dimensions, and upstream tariffs were checked against
OpenRouter on 2026-11-05. Each entry links to its upstream model. These catalog
prices feed usage accounting. Audio prices use integer micro-USD per million
input characters, UTF-8 bytes, or audio seconds. Deploy the Hub's catalog audio
tariff support before relying on these rates; older servers use their built-in
audio tariff. Models without a voice catalog use the provider's default voice.
Speech clients must honor a Bit's `max_input_characters` when splitting text.

Image prices reserve the highest supported per-image charge, including resolution
and quality variants. OpenRouter's reported cost determines the final charge when
available. Embedding Bits include model dimensions, input limits, and retrieval
prefixes. Free variants remain subject to upstream capacity and usage limits.

The bundle excludes PCM-only speech, audio priced by output duration or tokens,
image models that require reference images or SVG output, image models without a
bounded output tariff, batch-only embeddings, and models with no available
endpoint. Review availability, retirement dates, and rates when updating it.
