# Flow-Like WASM SDK for MoonBit

SDK library for building Flow-Like WASM nodes in [MoonBit](https://www.moonbitlang.com/).

## Modules

| File | Purpose |
|------|---------|
| `types.mbt` | Type definitions, enums, builder patterns, JSON serialization |
| `json.mbt` | Self-contained JSON parser |
| `host.mbt` | Raw FFI host imports and high-level wrappers |
| `context.mbt` | Execution context with input/output helpers |
| `memory.mbt` | Linear memory allocator, UTF-8 codec, pack/unpack |

## Usage

Current template setup (local sibling dependency):

```json
{
  "deps": {
    "flowlike/wasm-sdk-moonbit": { "path": "../wasm-sdk-moonbit" }
  }
}
```

After publishing to mooncakes, switch to a registry dependency:

```json
{
  "deps": {
    "flowlike/wasm-sdk-moonbit": "0.1.0"
  }
}
```

Then import it in your `moon.pkg.json`:

```json
{
  "import": [
    { "path": "flowlike/wasm-sdk-moonbit", "alias": "sdk" }
  ]
}
```

Access SDK types and functions via the `@sdk` prefix:

```moonbit
let def = @sdk.NodeDefinition::new("my_node", "My Node", "Does things", "Custom/WASM")
def.add_pin(@sdk.input_pin("exec", "Execute", "Trigger", @sdk.data_type_exec()))
```

## Publishing

- Tag a release with `wasm-sdk-moonbit-v*`
- Set `MOONCAKES_TOKEN` in GitHub repository secrets
- Run the workflow at `.github/workflows/publish.yml`

## Host API Coverage

| Feature | Status |
|---------|--------|
| log | ✅ |
| pins | ✅ |
| vars | ✅ |
| cache | ✅ |
| meta | ✅ |
| stream | ✅ |
| storage | ✅ |
| models | ✅ |
| http | ✅ |
| auth | ✅ |
