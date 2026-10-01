Open `node.ts` from Practice Files. Its definition contains execution pins `exec` and `exec_out`, String input `text`, Boolean inputs `trim`/`lowercase`, String output `result` and Boolean output `changed`.

The run function reads options, transforms text and sets both outputs:

```typescript
const text = ctx.getString("text", "") ?? "";
let result = text;
if (ctx.getBool("trim", true) ?? true) result = result.trim();
if (ctx.getBool("lowercase", true) ?? true) result = result.toLowerCase();
ctx.setOutput("result", result);
ctx.setOutput("changed", result !== text);
return ctx.success();
```

In the current TypeScript SDK, **success() activates `exec_out`** and returns the result. **finish()** returns the accumulated result without that automatic activation. Use `activateExec` when selecting another explicit execution branch, and test the returned activated pins. Producing a value alone does not choose a branch.

The manifest sets package identity and resource configuration. The node definition declares protected capabilities. Keep the maintained template's bridge/build files; this lab changes the node implementation and its tests.

Before building, predict the output for `" Ab "` with trim disabled and lowercase enabled. Expect `" ab "`, with changed=true. Turning both options off should preserve the original and report changed=false.
