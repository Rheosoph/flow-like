import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createInterface } from "node:readline";
import { webcrypto } from "node:crypto";

globalThis.crypto ??= webcrypto;
const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();
const receive = async () => JSON.parse((await lines.next()).value);
const send = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const generated = new URL("../../../apps/web/public/device-crypto/flow_like_device_crypto.js", import.meta.url);
const crypto = await import(generated.href);
await crypto.default({ module_or_path: await readFile(new URL("flow_like_device_crypto_bg.wasm", generated)) });
const password = new TextEncoder().encode("native browser tunnel test password");
const vault = crypto.createControllerVault("device-tunnel", password);
const controller = crypto.unlockControllerVault("device-tunnel", password, Uint8Array.from(vault.vault));
password.fill(0);
send(controller.publicBundle().controller_key);
const deviceKey = Uint8Array.from((await receive()).management_key);
const handshake = controller.beginTunnelNoise("owner", deviceKey, 100);
send({ certificate: handshake.certificate(), data: Buffer.from(handshake.write(100)).toString("base64url") });
handshake.read(Buffer.from((await receive()).data, "base64url"), 100);
send({ data: Buffer.from(handshake.write(100)).toString("base64url") });
let session = handshake.finish(100);

function frame(kind, sequence, body, stream = 0) {
  const result = new Uint8Array(20 + body.length);
  result.set([70, 76, 84, 78, 1, kind, 0, 0]);
  const view = new DataView(result.buffer);
  view.setUint32(8, stream);
  view.setBigUint64(12, BigInt(sequence));
  result.set(body, 20);
  return result;
}
const encrypt = (bytes, now) => send({ data: Buffer.from(session.encrypt(bytes, now)).toString("base64url") });
const decrypt = async (now) => session.decrypt(Buffer.from((await receive()).data, "base64url"), now);
const first = frame(3, 0, new TextEncoder().encode("epoch one"), 1);
encrypt(first, 100);
assert.deepEqual(await decrypt(100), first);

const renewal = controller.beginTunnelNoise("owner", deviceKey, 340);
encrypt(frame(9, 1, new TextEncoder().encode(JSON.stringify({
  certificate_jws: renewal.certificate(),
  data: Buffer.from(renewal.write(340)).toString("base64url"),
}))), 340);
const reply = await decrypt(340);
assert.equal(reply[5], 10);
assert.equal(new DataView(reply.buffer).getBigUint64(12), 1n);
renewal.read(reply.subarray(20), 340);
encrypt(frame(11, 2, renewal.write(340)), 340);
const previous = session;
session = renewal.finish(340);
previous.close();
previous.free();
const ready = await decrypt(340);
assert.equal(ready[5], 12);
assert.equal(new DataView(ready.buffer).getBigUint64(12), 2n);
assert.equal(JSON.parse(new TextDecoder().decode(ready.subarray(20))).expires_at, 640);
const after = frame(3, 3, new Uint8Array(16_364).fill(42), 1);
encrypt(after, 401);
assert.deepEqual(await decrypt(401), after);
encrypt(frame(3, 4, new Uint8Array([1]), 1), 401);
encrypt(frame(3, 5, new Uint8Array([2]), 1), 401);
assert.equal((await receive()).closed, true);
controller.close();
assert.throws(() => session.encrypt(new Uint8Array([1]), 401));
session.free();
controller.free();
