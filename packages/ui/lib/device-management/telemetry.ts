import { base64url, unbase64url } from "./crypto";
import {
	type DeviceAccountScope,
	commitMlsSnapshot,
	readMlsSnapshot,
} from "./storage";
import type {
	BrowserController,
	BrowserMlsEndpoint,
	DeviceReceipt,
	ManagementResponse,
	MlsDelivery,
	OnboardingManifest,
	TelemetryMember,
} from "./types";

export type ManagementCall = (
	command: Record<string, unknown>,
	operationId?: string,
) => Promise<ManagementResponse>;
const decoder = new TextDecoder("utf-8", { fatal: true });
export async function digestText(value: string): Promise<string> {
	return base64url(
		new Uint8Array(
			await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value)),
		),
	);
}

/** Pin every chunk to its first digest, length and sequence before parsing. */
export async function readTelemetryChunks(
	call: ManagementCall,
	command: Record<string, unknown>,
	maximum = 2 * 1024 * 1024,
): Promise<{ text: string | null; latest: number; sequence: number }> {
	let request = command;
	let offset = 0;
	let total = 0;
	let digest = "";
	let sequence = 0;
	let latest = 0;
	let bytes: Uint8Array | undefined;
	do {
		const response = await call({ ...request, offset, limit: 4096 });
		if (response.state !== "completed")
			throw new Error(
				"The device could not return the requested group telemetry.",
			);
		const part = response.result;
		if (part.available === false) {
			if (offset) throw new Error("Group telemetry changed during transfer.");
			return {
				text: null,
				latest: Number.isSafeInteger(part.latest) ? Number(part.latest) : 0,
				sequence: 0,
			};
		}
		if (
			part.available !== true ||
			part.offset !== offset ||
			!Number.isSafeInteger(part.total) ||
			Number(part.total) <= 0 ||
			Number(part.total) > maximum ||
			typeof part.digest !== "string" ||
			!/^[A-Za-z0-9_-]{43}$/u.test(part.digest) ||
			typeof part.chunk !== "string"
		)
			throw new Error("Invalid group telemetry chunk.");
		if (offset === 0) {
			total = Number(part.total);
			digest = part.digest;
			sequence = Number.isSafeInteger(part.sequence)
				? Number(part.sequence)
				: 0;
			latest = Number.isSafeInteger(part.latest) ? Number(part.latest) : 0;
			bytes = new Uint8Array(total);
		} else if (
			total !== part.total ||
			digest !== part.digest ||
			(sequence !== 0 && sequence !== part.sequence)
		)
			throw new Error("Group telemetry changed during transfer.");
		const chunk = unbase64url(part.chunk, 4096);
		if (!chunk.length || offset + chunk.length > total)
			throw new Error("Invalid group telemetry chunk length.");
		bytes?.set(chunk, offset);
		offset += chunk.length;
		// Sequence zero selects the oldest publication. Pin subsequent requests.
		if (sequence && request.sequence === 0) request = { ...request, sequence };
	} while (offset < total);
	const text = decoder.decode(bytes);
	if ((await digestText(text)) !== digest)
		throw new Error("Group telemetry digest changed.");
	return { text, latest, sequence };
}

export class GroupMetricsReader {
	private constructor(
		private readonly endpoint: BrowserMlsEndpoint,
		private readonly account: DeviceAccountScope,
		private readonly device: string,
		private readonly endpointId: string,
		readonly scope: string,
	) {}
	static async open(
		controller: BrowserController,
		account: DeviceAccountScope,
		manifest: OnboardingManifest,
		receipt: DeviceReceipt,
		scope: string,
	): Promise<GroupMetricsReader> {
		const publicKey = controller.publicBundle();
		if (
			publicKey.device_id !== manifest.device_id ||
			receipt.device_id !== manifest.device_id
		)
			throw new Error("Group telemetry belongs to another device.");
		const audience = {
			scope,
			owner_invitation_key: manifest.owner_invitation_key,
			publisher: {
				endpoint_id: manifest.device_id,
				signing_key: receipt.identity.telemetry_key,
			},
		};
		const saved = await readMlsSnapshot(
			account,
			manifest.device_id,
			publicKey.endpoint_id,
			scope,
		);
		const endpoint = saved
			? controller.openTelemetry(audience, saved.snapshot, saved.checkpoint)
			: controller.createTelemetry(audience);
		const reader = new GroupMetricsReader(
			endpoint,
			account,
			manifest.device_id,
			publicKey.endpoint_id,
			scope,
		);
		try {
			if (!saved) await reader.commit();
			return reader;
		} catch (error) {
			reader.close();
			throw error;
		}
	}
	private commit() {
		return commitMlsSnapshot(
			this.account,
			this.device,
			this.endpointId,
			this.scope,
			this.endpoint,
		);
	}
	position() {
		return this.endpoint.position();
	}
	async confirmDeliveries(call: ManagementCall): Promise<void> {
		for (const [receipt, receipt_jws] of this.endpoint.deliveryReceipts()) {
			const response = await call({
				type: "telemetry_receipt",
				scope: this.scope,
				endpoint_id: receipt.endpoint_id,
				sequence: receipt.sequence,
				receipt_jws,
			});
			if (
				response.state !== "completed" ||
				response.result.sequence !== receipt.sequence ||
				response.result.endpoint_id !== receipt.endpoint_id
			)
				throw new Error(
					"MLS delivery receipt has no confirmed result. Reconnect and retry before reading further messages.",
				);
			this.endpoint.prepareReceiptConfirmation(
				receipt.sequence,
				receipt.envelope_digest,
			);
			await this.commit();
		}
	}
	async keyPackage(
		member: TelemetryMember,
	): Promise<{ member: TelemetryMember; key_package: string }> {
		this.endpoint.prepareKeyPackage();
		const result = (await this.commit()) as { kind: string; wire: string };
		if (result.kind !== "key_package" || typeof result.wire !== "string")
			throw new Error("MLS key package was not persisted.");
		return { member, key_package: result.wire };
	}
	async read(
		call: ManagementCall,
		joinSequence = 0,
	): Promise<{
		sequence: number;
		envelopeDigest: string;
		sample?: Record<string, unknown>;
		state: string;
		receiptPending: boolean;
	} | null> {
		await this.confirmDeliveries(call);
		const position = this.endpoint.position();
		if (position.retired)
			throw new Error(
				"This MLS endpoint was removed. Create a fresh browser endpoint before rejoining.",
			);
		const sequence = position.joined ? position.sequence + 1 : joinSequence;
		if (!Number.isSafeInteger(sequence) || sequence < 0)
			throw new Error("Enter a valid welcome sequence.");
		const transfer = await readTelemetryChunks(call, {
			type: "telemetry_read",
			scope: this.scope,
			sequence,
			welcome: !position.joined,
		});
		if (transfer.text === null) {
			if (position.joined && transfer.latest >= sequence)
				throw new Error(
					"This reader missed an evicted MLS message and needs fresh admission.",
				);
			return null;
		}
		const delivery = JSON.parse(transfer.text) as MlsDelivery;
		// The native wrapper validates this policy again before consuming a ratchet.
		if (
			typeof delivery.policy_jws !== "string" ||
			typeof delivery.envelope_jws !== "string"
		)
			throw new Error("Invalid MLS delivery.");
		const now = Math.floor(Date.now() / 1000);
		if (position.joined) this.endpoint.prepareReceive(delivery, now);
		else this.endpoint.prepareJoin(delivery, now);
		const result = (await this.commit()) as {
			kind: string;
			plaintext?: string;
		};
		let sample: Record<string, unknown> | undefined;
		if (result.kind === "application") {
			const plaintext = unbase64url(result.plaintext ?? "", 1024 * 1024);
			try {
				const value = JSON.parse(decoder.decode(plaintext)) as {
					type: string;
					scope: string;
					sample: Record<string, unknown>;
				};
				if (value.type !== "metrics" || value.scope !== this.scope)
					throw new Error("Unexpected group telemetry audience.");
				sample = value.sample;
			} finally {
				plaintext.fill(0);
			}
		}
		let receiptPending = false;
		try {
			await this.confirmDeliveries(call);
		} catch {
			receiptPending = true;
		}
		return {
			sequence: transfer.sequence,
			envelopeDigest: await digestText(delivery.envelope_jws),
			sample,
			state: result.kind,
			receiptPending,
		};
	}
	close() {
		this.endpoint.close();
		this.endpoint.free();
	}
}

/** Owner action only, after every intended reader has durably consumed the item. */
export async function acknowledgeGroupTelemetry(
	call: ManagementCall,
	scope: string,
	sequence: number,
	envelopeDigest: string,
): Promise<void> {
	const result = await call({
		type: "telemetry_acknowledge",
		scope,
		sequence,
		envelope_digest: envelopeDigest,
	});
	if (result.state !== "completed")
		throw new Error("The device did not acknowledge this group delivery.");
}
