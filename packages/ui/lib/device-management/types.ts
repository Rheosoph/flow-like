import type {
	CertificateAuthoritySpec,
	CertificateAuthorityEnvelope,
	CertificateAuthorityPublic,
	CertificateAuthoritySigningRequest,
	SignedCertificateChain,
} from "./certificate-authority";
export interface Ed25519PublicKey {
	kty: "OKP";
	crv: "Ed25519";
	x: string;
}
export interface TelemetryMember {
	endpoint_id: string;
	signing_key: Ed25519PublicKey;
}
export interface ControllerPublic {
	device_id: string;
	endpoint_id: string;
	controller_key: Ed25519PublicKey;
	archive_key: number[];
	telemetry_member: TelemetryMember;
}
export interface OnboardingManifest {
	version: number;
	enrollment_id: string;
	device_id: string;
	owner_id: string;
	name: string;
	api_base_url: string;
	bootstrap_key: Ed25519PublicKey;
	controller_key: Ed25519PublicKey;
	owner_invitation_key: Ed25519PublicKey;
	issued_at: number;
	expires_at: number;
}
export interface DeviceReceipt {
	enrollment_id: string;
	device_id: string;
	owner_id: string;
	name: string;
	identity: {
		auth_key: Ed25519PublicKey;
		telemetry_key: Ed25519PublicKey;
		management_key: number[];
	};
	manifest_jws: string;
	binding_jws: string;
	registered_at: number;
	auth_epoch: number;
}
export interface Checkpoint {
	store_id: number[];
	revision: number;
	digest: string;
}
export interface ProtectedSnapshot {
	store_id: number[];
	revision: number;
	ciphertext: string;
}
export interface MlsPreparation {
	previous_checkpoint: Checkpoint | null;
	checkpoint: Checkpoint;
	snapshot: ProtectedSnapshot;
}
export interface BrowserMlsEndpoint {
	deliveryReceipts(): [
		{
			version: number;
			device_id: string;
			scope: string;
			endpoint_id: string;
			sequence: number;
			policy_digest: string;
			envelope_digest: string;
		},
		string,
	][];
	prepareReceiptConfirmation(sequence: number, envelopeDigest: string): void;
	position(): { joined: boolean; retired: boolean; sequence: number };
	preparedSnapshot(): MlsPreparation;
	checkpoint(): Checkpoint | null;
	confirmCommit(checkpoint: Checkpoint): unknown;
	discardPrepared(): void;
	prepareKeyPackage(): void;
	prepareJoin(delivery: unknown, now: number): void;
	prepareReceive(delivery: unknown, now: number): void;
	close(): void;
	free(): void;
}
export interface NoiseSession {
	encrypt(bytes: Uint8Array, now: number): Uint8Array;
	decrypt(bytes: Uint8Array, now: number): Uint8Array;
	close(): void;
	free(): void;
}
export interface NoiseHandshake {
	certificate(): string;
	sessionId(): string;
	write(now: number): Uint8Array;
	read(bytes: Uint8Array, now: number): void;
	finish(now: number): NoiseSession;
	close(): void;
	free(): void;
}
export interface ControllerVault {
	public_bundle: ControllerPublic;
	vault: number[];
}
export interface InvitationVault {
	public_key: Ed25519PublicKey;
	vault: number[];
}
export interface RewrappedControllerVaults {
	controller: ControllerVault;
	invitation?: InvitationVault | null;
}
export interface BrowserController {
	verifyFleetReader(
		trusted: FleetTrustedContext,
		receipt: DeviceReceipt,
		readerJws: string,
	): { revision: number; issued_at: number; expires_at: number };
	createFleetReader(
		api: string,
		user: string,
		revision: bigint,
		issuedAt: bigint,
		expiresAt: bigint,
	): string;
	verifyFleetView(
		trusted: FleetTrustedContext,
		receipt: DeviceReceipt,
		view: FleetView,
		now: bigint,
	): {
		reader_revision: number;
		reader_expires_at: number;
		audiences: FleetAudience[];
	};
	openFleet(
		trusted: FleetTrustedContext,
		receipt: DeviceReceipt,
		view: FleetView,
		bundle: EncryptedFleetSnapshot,
		now: bigint,
	): Uint8Array;
	sealInventory(
		binding: InventoryBinding,
		plaintext: Uint8Array,
	): EncryptedInventory;
	openInventory(
		expected: InventoryBinding,
		encrypted: EncryptedInventory,
	): Uint8Array;
	openArchive(
		pins: ArchivePins,
		bundle: EncryptedArchive,
		recipientId: string,
	): Uint8Array;
	freshEndpointVault(password: Uint8Array): ControllerVault;
	publicBundle(): ControllerPublic;
	signOnboardingManifest(manifest: OnboardingManifest): string;
	completeOnboarding(
		manifest: OnboardingManifest,
		password: Uint8Array,
		invitationVault: Uint8Array,
	): {
		controller: ControllerVault;
		invitation_vault: number[];
		manifest_jws: string;
	};
	beginNoise(
		grantId: string,
		deviceKey: Uint8Array,
		now: number,
	): NoiseHandshake;
	createTelemetry(audience: unknown): BrowserMlsEndpoint;
	openTelemetry(
		audience: unknown,
		snapshot: ProtectedSnapshot,
		checkpoint: Checkpoint,
	): BrowserMlsEndpoint;
	close(): void;
	free(): void;
}
export interface DeviceCrypto {
	createCertificateAuthorityVault(
		spec: CertificateAuthoritySpec,
		password: Uint8Array,
		now: number,
	): CertificateAuthorityEnvelope;
	inspectCertificateAuthorityVault(
		accountBinding: string,
		authorityId: string,
		password: Uint8Array,
		vault: Uint8Array,
	): CertificateAuthorityPublic;
	inspectCertificateAuthorityBackup(
		accountBinding: string,
		authorityId: string,
		password: Uint8Array,
		vault: Uint8Array,
		rootVault: Uint8Array,
	): CertificateAuthorityPublic;
	rewrapCertificateAuthorityVault(
		accountBinding: string,
		authorityId: string,
		currentPassword: Uint8Array,
		newPassword: Uint8Array,
		vault: Uint8Array,
		rootVault: Uint8Array,
	): CertificateAuthorityEnvelope;
	renewCertificateAuthorityVault(
		accountBinding: string,
		authorityId: string,
		password: Uint8Array,
		vault: Uint8Array,
		rootVault: Uint8Array,
		now: number,
	): CertificateAuthorityEnvelope;
	signServiceCertificate(
		accountBinding: string,
		authorityId: string,
		password: Uint8Array,
		vault: Uint8Array,
		request: CertificateAuthoritySigningRequest,
		now: number,
	): SignedCertificateChain;
	signDeviceCertificateIssuer(
		accountBinding: string,
		authorityId: string,
		password: Uint8Array,
		vault: Uint8Array,
		request: CertificateAuthoritySigningRequest,
		now: number,
	): SignedCertificateChain;
	sealAccountRecovery(
		scope: AccountRecoveryContext,
		password: Uint8Array,
		backup: Uint8Array,
	): { ciphertext: number[]; proof_jws: string };
	openAccountRecovery(
		scope: AccountRecoveryContext,
		password: Uint8Array,
		ciphertext: Uint8Array,
	): unknown;
	rewrapControllerVaults(
		deviceId: string,
		currentPassword: Uint8Array,
		newPassword: Uint8Array,
		controllerVault: Uint8Array,
		invitationVault: Uint8Array,
	): RewrappedControllerVaults;
	signArchiveRoster(
		roster: ArchiveRoster,
		password: Uint8Array,
		invitationVault: Uint8Array,
	): string;
	verifyArchiveRosterHead(
		compact: string,
		ownerKey: Ed25519PublicKey,
	): ArchiveRoster;
	verifyHistoricalTelemetryRoster(
		compact: string,
		ownerKey: Ed25519PublicKey,
	): TelemetryRoster;
	signTelemetryRoster(
		roster: TelemetryRoster,
		password: Uint8Array,
		invitationVault: Uint8Array,
	): string;
	verifyTelemetryRoster(
		compact: string,
		ownerKey: Ed25519PublicKey,
		now: number,
	): TelemetryRoster;
	default(): Promise<unknown>;
	createControllerVault(
		deviceId: string,
		password: Uint8Array,
	): ControllerVault;
	createInvitationVault(
		deviceId: string,
		password: Uint8Array,
	): InvitationVault;
	createOnboardingVaults(password: Uint8Array): {
		controller: ControllerVault;
		invitation: InvitationVault;
	};
	createBootstrapKey(): { public_key: Ed25519PublicKey; secret_base64: string };
	unlockControllerVault(
		deviceId: string,
		password: Uint8Array,
		ciphertext: Uint8Array,
	): BrowserController;
	verifyDeviceReceipt(
		receipt: DeviceReceipt,
		expectedManifest: string,
		controllerKey: Ed25519PublicKey,
	): OnboardingManifest;
	signManagementPolicy(
		policy: ManagementPolicy,
		password: Uint8Array,
		invitationVault: Uint8Array,
	): string;
	verifyManagementPolicy(
		compact: string,
		ownerKey: Ed25519PublicKey,
	): ManagementPolicy;
}
export type Capability =
	| "status"
	| "logs"
	| "metrics"
	| "deploy"
	| "start"
	| "stop"
	| "restart"
	| "remove"
	| "scale"
	| "update_agent"
	| "manage_certificates"
	| "reboot";

export interface TelemetryRoster {
	version: 1;
	device_id: string;
	scope: string;
	policy_version: number;
	previous_policy_digest: string | null;
	management_policy_digest: string | null;
	publisher: TelemetryMember;
	members: TelemetryMember[];
	issued_at: number;
	expires_at: number;
}
export interface MlsDelivery {
	envelope_jws: string;
	policy_jws: string;
	wire: string;
}

export interface ArchiveRecipient {
	recipient_id: string;
	user_id: string;
	public_key: number[];
}
export interface ArchiveRoster {
	version: 1;
	device_id: string;
	scope: string;
	project_id: string | null;
	kind: "logs" | "metrics";
	policy_version: number;
	previous_policy_digest: string | null;
	management_policy_digest: string | null;
	recipients: ArchiveRecipient[];
	issued_at: number;
	expires_at: number;
}
export interface ArchivePins {
	device_id: string;
	scope: string;
	kind: "logs" | "metrics";
	owner_invitation_key: Ed25519PublicKey;
	device_signing_key: Ed25519PublicKey;
}
export interface EncryptedArchive {
	roster_jws: string;
	manifest_jws: string;
	ciphertext: string;
	recipient_keys: { recipient_id: string; wrapped_key: string }[];
}
export interface ManagementGrant {
	grant_id: string;
	user_id: string;
	controller_key: Ed25519PublicKey;
	scope:
		| { kind: "device" }
		| { kind: "project"; project_id: string }
		| { kind: "placement"; project_id: string; placement_id: string };
	capabilities: Capability[];
	expires_at: number;
	group_id: string | null;
	group_version: number | null;
}
export interface ManagementPolicy {
	version: 1;
	device_id: string;
	policy_version: number;
	previous_policy_digest: string | null;
	grants: ManagementGrant[];
	issued_at: number;
	expires_at: number;
}
export interface PolicyView {
	policy_jws: string | null;
	version: number;
	digest: string | null;
	applied_version: number;
	applied_digest: string | null;
}
export interface SignalingAdmission {
	token: string;
	expires_at: number;
	device_auth_epoch: number;
	signaling_urls: string[];
	ice_servers: RTCIceServer[];
	ice_expires_at: number | null;
	policy_version: number;
	policy_digest: string | null;
}
export interface ManagementResponse {
	operation_id: string;
	state: string;
	result: Record<string, unknown>;
}
export interface PlacementStatus {
	id: string;
	project_id: string;
	deployment_id: string;
	revision: string;
	desired_state: string;
	observed_state: string;
	config_revision: number;
	intent_revision: number;
	applied_revision: number | null;
	desired_replicas?: number;
	running_replicas?: number;
	ready_replicas?: number;
	max_replicas?: number;
	replicas?: {
		slot: number;
		observed_state: string;
		applied_revision: number | null;
	}[];
}
export interface Inspection {
	device_id: string;
	boot_id: string | null;
	placements: PlacementStatus[];
	observed_at?: number;
	certificate_management?: 1;
	can_manage_certificates?: boolean;
	certificate_issuance?: 1;
	certificate_acme?: 1;
	can_delegate_certificate_renewal?: boolean;
}

export type InventoryScope = ManagementGrant["scope"];
export interface InventoryBinding {
	issuer: string;
	api_origin: string;
	account_id: string;
	device_id: string;
	controller_key: Ed25519PublicKey;
	scope: InventoryScope;
	revision: number;
}
export interface EncryptedInventory {
	binding: InventoryBinding;
	ciphertext: string;
}
export interface InventoryView {
	scopes: InventoryScope[];
	observations: EncryptedInventory[];
}

export interface AccountRecoveryContext {
	issuer: string;
	account: string;
	api_origin: string;
	device_id: string;
	controller_key: Ed25519PublicKey;
	revision: number;
}
export interface AccountRecoveryWrite {
	public_key: Ed25519PublicKey;
	ciphertext: string;
	revision: number;
	proof_jws: string;
}

export interface FleetTrustedContext {
	api_base_url: string;
	user_id: string;
	onboarding_manifest_jws: string;
	owner_controller_key: Ed25519PublicKey;
}
export interface FleetAudience {
	reader_digest: string;
	grant_id: string;
	scope: InventoryScope;
	kind: "status" | "metrics";
	policy_digest: string | null;
}
export interface EncryptedFleetSnapshot {
	manifest_jws: string;
	ciphertext: string;
}
export interface FleetView {
	snapshots: EncryptedFleetSnapshot[];
	policy_jws: string | null;
	reader_jws: string;
}
export interface FleetReaderState {
	reader_jws: string;
	revision: number;
	deleted: boolean;
}
export interface FleetLocalState {
	revision: number;
	readerJws?: string;
	pending?: { reader_jws: string; revision: number };
	policyVersion?: number;
	policyDigest?: string;
	anchors: Record<
		string,
		{
			sequence: number;
			digest: string;
			scope: InventoryScope;
			kind: "status" | "metrics";
			grant_id: string;
			reader_digest: string;
			policy_digest: string | null;
		}
	>;
}
