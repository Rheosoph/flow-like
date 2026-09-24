import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { webcrypto, createHash, createPrivateKey, sign } from "node:crypto";

globalThis.crypto ??= webcrypto;
const generated = new URL("../../../apps/web/public/device-crypto/flow_like_device_crypto.js", import.meta.url);
const crypto = await import(generated.href);
await crypto.default({ module_or_path: await readFile(new URL("flow_like_device_crypto_bg.wasm", generated)) });

test("generated browser WASM preserves long-term keys and replaces restored MLS endpoints", () => {
  const password = new TextEncoder().encode("browser integration test password");
  const created = crypto.createControllerVault("device-test", password);
  const invitation = crypto.createInvitationVault("device-test", password);
  assert.throws(() => crypto.unlockControllerVault("other-device", password, Uint8Array.from(created.vault)));
  const unlocked = crypto.unlockControllerVault("device-test", password, Uint8Array.from(created.vault));
  assert.deepEqual(unlocked.publicBundle(), created.public_bundle);
  const endpoint = unlocked.createTelemetry({
    scope: "device", owner_invitation_key: invitation.public_key,
    publisher: { endpoint_id: "device-test", signing_key: crypto.createBootstrapKey().public_key },
  });
  const initial = endpoint.preparedSnapshot();
  assert.throws(() => endpoint.prepareKeyPackage());
  endpoint.confirmCommit(initial.checkpoint);
  endpoint.prepareKeyPackage();
  const pending = endpoint.preparedSnapshot();
  assert.notDeepEqual(pending.checkpoint, initial.checkpoint);
  const publishedPackage = endpoint.confirmCommit(pending.checkpoint);
  assert.equal(publishedPackage.kind, "key_package");
  assert.ok(Buffer.from(publishedPackage.wire, "base64url").length > 32);
  const restored = unlocked.freshEndpointVault(password);
  assert.deepEqual(restored.public_bundle.controller_key, created.public_bundle.controller_key);
  assert.deepEqual(restored.public_bundle.archive_key, created.public_bundle.archive_key);
  assert.notEqual(restored.public_bundle.endpoint_id, created.public_bundle.endpoint_id);
  assert.notDeepEqual(restored.public_bundle.telemetry_member, created.public_bundle.telemetry_member);
  assert.throws(() => endpoint.position());
  const resumed = crypto.unlockControllerVault("device-test", password, Uint8Array.from(restored.vault));
  assert.deepEqual(resumed.publicBundle(), restored.public_bundle);
  resumed.close();
  assert.throws(() => resumed.publicBundle());
  unlocked.close();
  password.fill(0);
});

test("generated browser WASM rewraps both password vaults and reopens the same MLS snapshot", () => {
  const previous = new TextEncoder().encode("previous browser integration password");
  const replacement = new TextEncoder().encode("replacement browser integration password");
  const handles = [];
  try {
    const created = crypto.createControllerVault("device-password", previous);
    const invitation = crypto.createInvitationVault("device-password", previous);
    const controllerCiphertext = Uint8Array.from(created.vault);
    const invitationCiphertext = Uint8Array.from(invitation.vault);
    const original = crypto.unlockControllerVault("device-password", previous, controllerCiphertext);
    handles.push(original);
    const audience = {
      scope: "device",
      owner_invitation_key: invitation.public_key,
      publisher: { endpoint_id: "device-password", signing_key: crypto.createBootstrapKey().public_key },
    };
    const endpoint = original.createTelemetry(audience);
    handles.push(endpoint);
    endpoint.confirmCommit(endpoint.preparedSnapshot().checkpoint);
    endpoint.prepareKeyPackage();
    const persisted = endpoint.preparedSnapshot();
    endpoint.confirmCommit(persisted.checkpoint);
    const position = endpoint.position();
    endpoint.close();
    original.close();
    const policy = {
      version: 1, device_id: "device-password", policy_version: 1,
      previous_policy_digest: null, grants: [], issued_at: 100, expires_at: 200,
    };
    const signedBefore = crypto.signManagementPolicy(policy, previous, invitationCiphertext);
    assert.throws(() => crypto.rewrapControllerVaults(
      "device-password", replacement, previous, controllerCiphertext, invitationCiphertext,
    ));
    assert.throws(() => crypto.rewrapControllerVaults(
      "device-password", previous, replacement, controllerCiphertext, controllerCiphertext,
    ));
    const changed = crypto.rewrapControllerVaults(
      "device-password", previous, replacement, controllerCiphertext, invitationCiphertext,
    );
    assert.deepEqual(changed.controller.public_bundle, created.public_bundle);
    assert.deepEqual(changed.invitation.public_key, invitation.public_key);
    assert.notDeepEqual(changed.controller.vault, created.vault);
    assert.notDeepEqual(changed.invitation.vault, invitation.vault);
    const nextController = Uint8Array.from(changed.controller.vault);
    const nextInvitation = Uint8Array.from(changed.invitation.vault);
    assert.throws(() => crypto.unlockControllerVault("device-password", previous, nextController));
    assert.throws(() => crypto.signManagementPolicy(policy, previous, nextInvitation));
    assert.throws(() => crypto.unlockControllerVault("device-password", replacement, nextInvitation));
    const resumed = crypto.unlockControllerVault("device-password", replacement, nextController);
    handles.push(resumed);
    assert.deepEqual(resumed.publicBundle(), created.public_bundle);
    const reopened = resumed.openTelemetry(audience, persisted.snapshot, persisted.checkpoint);
    handles.push(reopened);
    assert.deepEqual(reopened.checkpoint(), persisted.checkpoint);
    assert.deepEqual(reopened.position(), position);
    const signedAfter = crypto.signManagementPolicy(policy, replacement, nextInvitation);
    assert.equal(signedAfter, signedBefore);
    assert.equal(crypto.verifyManagementPolicy(signedAfter, invitation.public_key).device_id, "device-password");
    // Rewrapping local storage does not revoke previously exported encrypted backups.
    const backup = crypto.unlockControllerVault("device-password", previous, controllerCiphertext);
    handles.push(backup);
    assert.deepEqual(backup.publicBundle(), created.public_bundle);
    assert.equal(crypto.signManagementPolicy(policy, previous, invitationCiphertext), signedBefore);
  } finally {
    for (const handle of handles.reverse()) { handle.close(); handle.free(); }
    previous.fill(0);
    replacement.fill(0);
  }
});

test("generated browser WASM changes a shared user's password without invitation authority", () => {
  const previous = new TextEncoder().encode("previous shared browser password");
  const replacement = new TextEncoder().encode("replacement shared browser password");
  let unlocked;
  try {
    const created = crypto.createControllerVault("shared-device", previous);
    const changed = crypto.rewrapControllerVaults(
      "shared-device", previous, replacement, Uint8Array.from(created.vault), new Uint8Array(),
    );
    assert.ok(changed.invitation === undefined || changed.invitation === null);
    assert.deepEqual(changed.controller.public_bundle, created.public_bundle);
    assert.notDeepEqual(changed.controller.vault, created.vault);
    assert.throws(() => crypto.unlockControllerVault("shared-device", previous, Uint8Array.from(changed.controller.vault)));
    unlocked = crypto.unlockControllerVault("shared-device", replacement, Uint8Array.from(changed.controller.vault));
    assert.deepEqual(unlocked.publicBundle(), created.public_bundle);
  } finally {
    unlocked?.close();
    unlocked?.free();
    previous.fill(0);
    replacement.fill(0);
  }
});

test("generated browser WASM retains private inventory across password recovery and fresh endpoints", () => {
  const password = new TextEncoder().encode("inventory account recovery password");
  const wrong = new TextEncoder().encode("incorrect account recovery password");
  const handles = [];
  try {
    const initial = crypto.createOnboardingVaults(password);
    const controller = crypto.unlockControllerVault(initial.controller.public_bundle.device_id, password, Uint8Array.from(initial.controller.vault));
    handles.push(controller);
    const manifest = {
      version: 1, enrollment_id: "inventory-enrollment", device_id: "inventory-device", owner_id: "owner",
      name: "Inventory recovery test", api_base_url: "https://hub.example/api/v1",
      bootstrap_key: crypto.createBootstrapKey().public_key,
      controller_key: initial.controller.public_bundle.controller_key,
      owner_invitation_key: initial.invitation.public_key, issued_at: 100, expires_at: 200,
    };
    const completed = controller.completeOnboarding(manifest, password, Uint8Array.from(initial.invitation.vault));
    const context = { issuer: "https://auth.example", account: "owner", api_origin: "https://hub.example", device_id: manifest.device_id, controller_key: manifest.controller_key, revision: 1 };
    const binding = { issuer: context.issuer, account_id: context.account, api_origin: context.api_origin, device_id: context.device_id, controller_key: context.controller_key, scope: { kind: "project", project_id: "project" }, revision: 2 };
    const plaintext = new TextEncoder().encode(JSON.stringify({ device_id: manifest.device_id, placements: [{ id: "placement", project_id: "project", observed_state: "running" }], observed_at: 1000 }));
    const encrypted = controller.sealInventory(binding, plaintext);
    assert.equal(new TextDecoder().decode(controller.openInventory(binding, encrypted)), new TextDecoder().decode(plaintext));
    for (const changed of [{ account_id: "other" }, { api_origin: "https://other.example" }, { issuer: "https://other-auth.example" }, { device_id: "other-device" }, { revision: 1 }, { scope: {kind: "device"} }]) {
      assert.throws(() => controller.openInventory({ ...binding, ...changed }, encrypted));
      assert.throws(() => controller.openInventory({ ...binding, ...changed }, { ...encrypted, binding: { ...binding, ...changed } }));
    }
    const backup = {
      version: 1, apiOrigin: context.api_origin, deviceId: context.device_id,
      controllerPublic: completed.controller.public_bundle, controllerVault: Array.from(completed.controller.vault),
      invitationVault: Array.from(completed.invitation_vault), manifestJws: completed.manifest_jws,
      grantId: "owner", ownerControllerKey: null,
    };
    const encoded = new TextEncoder().encode(JSON.stringify(backup));
    const sealed = crypto.sealAccountRecovery(context, password, encoded);
    assert.equal(sealed.proof_jws.split(".").length, 3);
    const ciphertext = Uint8Array.from(sealed.ciphertext);
    assert.throws(() => crypto.openAccountRecovery(context, wrong, ciphertext));
    for (const changed of [{account:"other"}, {issuer:"https://elsewhere.example"}, {api_origin:"https://other.example"}, {device_id:"other-device"}, {revision:2}]) {
      assert.throws(() => crypto.openAccountRecovery({ ...context, ...changed }, password, ciphertext));
    }
    controller.close();
    const recovered = crypto.openAccountRecovery(context, password, ciphertext);
    const unlocked = crypto.unlockControllerVault(context.device_id, password, Uint8Array.from(recovered.controllerVault));
    handles.push(unlocked);
    const fresh = unlocked.freshEndpointVault(password);
    const reopened = crypto.unlockControllerVault(context.device_id, password, Uint8Array.from(fresh.vault));
    handles.push(reopened);
    assert.equal(new TextDecoder().decode(reopened.openInventory(binding, encrypted)), new TextDecoder().decode(plaintext));
    assert.notEqual(fresh.public_bundle.endpoint_id, completed.controller.public_bundle.endpoint_id);
    plaintext.fill(0); encoded.fill(0);
  } finally {
    for (const handle of handles.reverse()) { handle.close(); handle.free(); }
    password.fill(0); wrong.fill(0);
  }
});

test("generated browser WASM validates unattended fleet readers against the pinned device bootstrap", () => {
  const password = new TextEncoder().encode("fleet browser onboarding password");
  const handles = [];
  const digest = (bytes) => createHash("sha256").update(bytes).digest("base64url");
  const compact = (payload, type, key) => {
    const jwk = key.public_key;
    const kid = digest(JSON.stringify({crv:jwk.crv,kty:jwk.kty,x:jwk.x}));
    const input = `${Buffer.from(JSON.stringify({alg:"EdDSA",typ:type,kid})).toString("base64url")}.${Buffer.from(JSON.stringify(payload)).toString("base64url")}`;
    const privateKey = createPrivateKey({format:"der",type:"pkcs8",key:Buffer.concat([Buffer.from("302e020100300506032b657004220420","hex"),Buffer.from(key.secret_base64,"base64url")])});
    return `${input}.${sign(null,Buffer.from(input),privateKey).toString("base64url")}`;
  };
  try {
    const vaults = crypto.createOnboardingVaults(password);
    const controller = crypto.unlockControllerVault(vaults.controller.public_bundle.device_id,password,Uint8Array.from(vaults.controller.vault));
    handles.push(controller);
    const bootstrap = crypto.createBootstrapKey();
    const manifest = {version:1,enrollment_id:"fleet-enrollment",device_id:"fleet-device",owner_id:"owner",name:"Fleet test",api_base_url:"https://hub.example/api/v1",bootstrap_key:bootstrap.public_key,controller_key:vaults.controller.public_bundle.controller_key,owner_invitation_key:vaults.invitation.public_key,issued_at:100,expires_at:200};
    const completed = controller.completeOnboarding(manifest,password,Uint8Array.from(vaults.invitation.vault));
    const identity = {auth_key:crypto.createBootstrapKey().public_key,telemetry_key:crypto.createBootstrapKey().public_key,management_key:Array(32).fill(13)};
    const binding = {version:1,enrollment_id:manifest.enrollment_id,device_id:manifest.device_id,identity,manifest_digest:digest(completed.manifest_jws),challenge_id:"challenge",challenge_nonce:"abcdefghijklmnopqrstuvwx12345678",issued_at:100,expires_at:150};
    const receipt = {enrollment_id:manifest.enrollment_id,device_id:manifest.device_id,owner_id:manifest.owner_id,name:manifest.name,identity,manifest_jws:completed.manifest_jws,binding_jws:compact(binding,"flow-like-device-enrollment-binding+jwt",bootstrap),registered_at:110,auth_epoch:1};
    const trusted = {api_base_url:manifest.api_base_url,user_id:"owner",onboarding_manifest_jws:completed.manifest_jws,owner_controller_key:manifest.controller_key};
    const reader = controller.createFleetReader(manifest.api_base_url,"owner",1n,100n,300n);
    const view = {reader_jws:reader,policy_jws:null,snapshots:[]};
    assert.equal(controller.verifyFleetReader(trusted,receipt,reader).revision,1);
    assert.deepEqual(controller.verifyFleetView(trusted,receipt,view,120n).audiences.map((a)=>a.kind),["status","metrics"]);
    assert.throws(()=>controller.verifyFleetView(trusted,receipt,view,300n));
    assert.throws(()=>controller.verifyFleetReader({...trusted,user_id:"other"},receipt,reader));
    assert.throws(()=>controller.verifyFleetReader({...trusted,api_base_url:"https://other.example/api/v1"},receipt,reader));
    assert.throws(()=>controller.verifyFleetReader(trusted,{...receipt,identity:{...identity,telemetry_key:crypto.createBootstrapKey().public_key}},reader));
    assert.throws(()=>controller.verifyFleetReader({...trusted,owner_controller_key:crypto.createBootstrapKey().public_key},receipt,reader));
    const restored = controller.freshEndpointVault(password);
    const fresh = crypto.unlockControllerVault(manifest.device_id,password,Uint8Array.from(restored.vault));
    handles.push(fresh);
    assert.equal(fresh.verifyFleetView(trusted,receipt,view,120n).reader_revision,1);
    assert.equal(fresh.createFleetReader(manifest.api_base_url,"owner",1n,100n,300n),reader);
    const sharedVault = crypto.createControllerVault(manifest.device_id,password);
    const shared = crypto.unlockControllerVault(manifest.device_id,password,Uint8Array.from(sharedVault.vault));
    handles.push(shared);
    const sharedReader = shared.createFleetReader(manifest.api_base_url,"reader",1n,100n,300n);
    const sharedTrusted = {...trusted,user_id:"reader"};
    const sharedPolicy = {version:1,device_id:manifest.device_id,policy_version:1,previous_policy_digest:null,issued_at:100,expires_at:200,grants:[{grant_id:"status-only",user_id:"reader",controller_key:sharedVault.public_bundle.controller_key,scope:{kind:"project",project_id:"project"},capabilities:["status"],expires_at:190,group_id:null,group_version:null}]};
    const sharedView = {reader_jws:sharedReader,policy_jws:crypto.signManagementPolicy(sharedPolicy,password,Uint8Array.from(completed.invitation_vault)),snapshots:[]};
    assert.deepEqual(shared.verifyFleetView(sharedTrusted,receipt,sharedView,120n).audiences.map((a)=>a.kind),["status"]);
    assert.throws(()=>shared.verifyFleetView(sharedTrusted,receipt,{...sharedView,policy_jws:null},120n));
    assert.throws(()=>fresh.verifyFleetReader(sharedTrusted,receipt,sharedReader));
  } finally {
    for(const handle of handles.reverse()){handle.close();handle.free();}
    password.fill(0);
  }
});
