"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Archive,
	Check,
	CircleCheck,
	CircleDashed,
	Download,
	FileBadge,
	Hourglass,
	KeyRound,
	Laptop,
	OctagonX,
	Pencil,
	ShieldCheck,
	ShieldOff,
	Trash2,
	TriangleAlert,
	Upload,
} from "lucide-react";
import type { ReactNode } from "react";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import {
	type ChainLink,
	type ChainState,
	TrustChain,
} from "../primitives/trust-chain";
import {
	type AuthorityHome,
	type AuthorityNote,
	homeText,
} from "./authority-sheets";
import {
	AUTHORITY_ROOT_SOON_DAYS,
	AUTHORITY_SIGNING_SOON_DAYS,
	type AuthorityView,
	type CertificateRow,
	DAY_S,
	MAX_AUTHORITY_NAMES,
} from "./certificates-model";
import {
	DayOf,
	DeviceLink,
	LABEL,
	PROSE,
	certificateTitle,
	dayText,
	daysText,
	shortId,
	spanText,
	untilText,
} from "./parts";
import { saveFile } from "./use-certificates";

export type AuthorityAction =
	| "renew"
	| "password"
	| "test"
	| "restore"
	| "remove";
/** Where the signing key lives: a browser may clear its storage. */
export type AuthorityStorage = "desktop" | "web" | "web_unprotected";

/** The account, hub and profile an authority is stored for, and what kind of storage holds it. */
export interface AuthorityPlace {
	home: AuthorityHome;
	storage: AuthorityStorage;
}

type LifeTone = "good" | "warning" | "critical";

const LIFE_FILL: Record<LifeTone, string> = {
	good: "bg-good-solid",
	warning: "bg-warning-solid",
	critical: "bg-critical-solid",
};

const percent = (value: number) => `${value.toFixed(2)}%`;

/** Root and signing key on one scale from creation to the root's expiry, with today marked. */
function Lifetime({ view }: Readonly<{ view: AuthorityView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const start = view.createdAt;
	const span = Math.max(1, view.rootExpiresAt - start);
	const at = (moment: number) =>
		Math.max(0, Math.min(100, ((moment - start) / span) * 100));
	const today = at(time.nowS);
	const toneOf = (expiresAt: number, soonDays: number): LifeTone =>
		expiresAt <= time.nowS
			? "critical"
			: expiresAt - time.nowS <= soonDays * DAY_S
				? "warning"
				: "good";
	const rows = [
		{
			id: "root",
			label: t("certificates.authority.lifeRoot", "Root"),
			end: view.rootExpiresAt,
			tone: toneOf(view.rootExpiresAt, AUTHORITY_ROOT_SOON_DAYS),
		},
		{
			id: "signing",
			label: t("certificates.authority.lifeSigning", "Signing key"),
			end: view.signingExpiresAt,
			tone: toneOf(view.signingExpiresAt, AUTHORITY_SIGNING_SOON_DAYS),
		},
	];
	return (
		<div
			role="img"
			aria-label={t(
				"certificates.authority.lifeLabel",
				"Root valid from {{from}} to {{root}}. Signing key valid until {{signing}}.",
				{
					from: dayText(time, start),
					root: dayText(time, view.rootExpiresAt),
					signing: dayText(time, view.signingExpiresAt),
				},
			)}
			className="grid grid-cols-[76px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-2 text-xs @max-[560px]/devices:grid-cols-[64px_minmax(0,1fr)_auto]"
		>
			<span />
			<span
				aria-hidden
				className="relative h-3.5 text-[11px] leading-3.5 text-foreground"
			>
				<span
					className="absolute top-0 -translate-x-1/2 whitespace-nowrap"
					style={{ left: percent(today) }}
				>
					{t("certificates.authority.lifeToday", "today")}
				</span>
			</span>
			<span />
			{rows.map((row) => {
				const end = at(row.end);
				const now = Math.min(end, today);
				return (
					<div key={row.id} className="contents">
						<span className="whitespace-nowrap text-muted-foreground">
							{row.label}
						</span>
						<span
							data-life={row.tone}
							className="relative h-2 rounded-[2px] bg-muted"
						>
							<i
								className="absolute inset-y-0 left-0 rounded-l-[2px] bg-border-strong"
								style={{ width: percent(now) }}
							/>
							<i
								className={cx(
									"absolute inset-y-0 rounded-r-[2px]",
									LIFE_FILL[row.tone],
								)}
								style={{ left: percent(now), width: percent(end - now) }}
							/>
							<i
								className="absolute -top-1 -bottom-1 w-px bg-foreground"
								style={{ left: percent(today) }}
							/>
						</span>
						<DayOf
							at={row.end}
							className="font-mono font-medium whitespace-nowrap text-ink-2"
						/>
					</div>
				);
			})}
			<span />
			<span
				aria-hidden
				className="text-[11px] leading-3.5 text-muted-foreground"
			>
				{t("certificates.authority.lifeFrom", "from {{date}}", {
					date: dayText(time, start),
				})}
			</span>
			<span />
		</div>
	);
}

function StatusChips({
	view,
	restored,
}: Readonly<{ view: AuthorityView; restored: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (view.status === "root_expired")
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{t("certificates.authority.rootExpired", "Root expired")}
			</StatusChip>
		);
	if (view.status === "signing_expired")
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{t("certificates.authority.signingExpired", "Signing key expired")}
			</StatusChip>
		);
	return (
		<>
			{restored ? (
				<StatusChip tone="info" icon={Upload}>
					{t("certificates.authority.restored", "Restored")}
				</StatusChip>
			) : (
				<StatusChip tone="good" icon={CircleCheck}>
					{t("certificates.authority.active", "Active")}
				</StatusChip>
			)}
			{view.signingSoon ? (
				<StatusChip tone="warning" icon={Hourglass}>
					{t("certificates.authority.stopsIn", "Stops signing in {{span}}", {
						span: spanText(t, view.signingExpiresAt - time.nowS),
					})}
				</StatusChip>
			) : null}
		</>
	);
}

/** "expires 14 Mar 2027 · in 165d", with the full moment on the date's hover. */
function ExpiresOn({ at }: Readonly<{ at: number }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const values = { distance: untilText(time, at) };
	const components = { 1: <DayOf at={at} /> };
	return at <= time.nowS ? (
		<Trans
			t={t}
			i18nKey="certificates.authority.expiredOn"
			defaults="expired <1/> · {{distance}}"
			values={values}
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="certificates.authority.expiresOn"
			defaults="expires <1/> · {{distance}}"
			values={values}
			components={components}
		/>
	);
}

function Hint({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span className="mt-0.5 block text-xs text-muted-foreground">
			{children}
		</span>
	);
}

function Warning({
	view,
	onRenew,
}: Readonly<{ view: AuthorityView; onRenew(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const renew = (
		<DvButton size="sm" icon={KeyRound} onClick={onRenew}>
			{t("certificates.authority.renewKey", "Renew signing key…")}
		</DvButton>
	);
	if (view.status === "root_expired")
		return (
			<Banner
				tone="critical"
				title={t(
					"certificates.authority.rootExpiredTitle",
					"The root of {{label}} expired {{when}}.",
					{ label: view.label, when: time.at(view.rootExpiresAt) },
				)}
			>
				{t(
					"certificates.authority.rootExpiredText",
					"Clients no longer trust what it signed. Create a new authority and install its root on clients.",
				)}
			</Banner>
		);
	if (view.status === "signing_expired")
		return (
			<Banner
				tone="critical"
				title={t(
					"certificates.authority.signingExpiredTitle",
					"{{label}} can't sign any more: its signing key expired {{when}}.",
					{ label: view.label, when: time.at(view.signingExpiresAt) },
				)}
				actions={renew}
			>
				{t(
					"certificates.authority.signingExpiredText",
					"Requests and renewal authorities can't be signed until you renew the signing key from the backup file.",
				)}
			</Banner>
		);
	if (view.signingSoon)
		return (
			<Banner
				tone="warning"
				title={t(
					"certificates.authority.signingSoonTitle",
					"{{label}} stops signing on {{date}}, in {{days}}.",
					{
						label: view.label,
						date: dayText(time, view.signingExpiresAt),
						days: daysText(t, (view.signingExpiresAt - time.nowS) / DAY_S),
					},
				)}
				actions={renew}
			>
				{t(
					"certificates.authority.signingSoonText",
					"After that it can't sign requests or renewal authorities. Certificates it already signed keep working until their own expiry. Renew the signing key with your backup file and the authority password.",
				)}
			</Banner>
		);
	if (view.rootSoon)
		return (
			<Banner
				tone="warning"
				title={t(
					"certificates.authority.rootSoonTitle",
					"The root of {{label}} expires on {{date}}.",
					{ label: view.label, date: dayText(time, view.rootExpiresAt) },
				)}
			>
				{t(
					"certificates.authority.rootSoonText",
					"Certificates it signed stop being trusted then. Plan a new authority and install its root on clients before that date.",
				)}
			</Banner>
		);
	return null;
}

interface FactsProps {
	view: AuthorityView;
	/** Restored on this computer in this session. */
	restored: boolean;
	/** Certificates live reads attribute to this authority. */
	signed: readonly CertificateRow[];
}

function AuthorityFacts({ view, restored, signed }: Readonly<FactsProps>) {
	const { t } = useTranslation("devices");
	const names = [...view.suffixes, ...view.addresses];
	return (
		<KeyValueList>
			<KvRow label={t("certificates.authority.allowedNames", "Allowed names")}>
				<span className="flex flex-wrap gap-1">
					{names.length
						? names.map((name) => (
								<StatusChip key={name} tone="outline" className="font-mono">
									{name}
								</StatusChip>
							))
						: t("certificates.authority.noNames", "None")}
				</span>
				<Hint>
					{t(
						"certificates.authority.allowedNamesHint",
						"DNS suffixes include their subdomains; IP addresses match exactly. {{count, number}} of {{max, number}} names.",
						{ count: names.length, max: MAX_AUTHORITY_NAMES },
					)}
				</Hint>
			</KvRow>
			<KvRow label={t("certificates.authority.signingKey", "Signing key")}>
				<ExpiresOn at={view.signingExpiresAt} />
				<Hint>
					{t(
						"certificates.authority.signingKeyHint",
						"Signs requests and renewal authorities. It lasts up to 1 year; renew it from the backup.",
					)}
				</Hint>
			</KvRow>
			<KvRow label={t("certificates.authority.root", "Root")}>
				<ExpiresOn at={view.rootExpiresAt} />
				<Hint>
					{t(
						"certificates.authority.rootHint",
						"Clients trust the root. When it expires, everything it signed stops being trusted.",
					)}
				</Hint>
			</KvRow>
			<KvRow
				label={t("certificates.authority.rootFingerprint", "Root fingerprint")}
			>
				<IdRef
					id={view.fingerprint}
					group4
					copyLabel={t(
						"certificates.authority.copyFingerprint",
						"Copy root fingerprint",
					)}
				/>
				<Hint>
					{t(
						"certificates.authority.rootFingerprintHint",
						"Compare it with the root certificate installed on clients.",
					)}
				</Hint>
			</KvRow>
			<KvRow label={t("certificates.authority.id", "Authority ID")}>
				<IdRef
					id={view.id}
					copyLabel={t("certificates.authority.copyId", "Copy authority ID")}
				/>
			</KvRow>
			<KvRow
				label={t("certificates.authority.created", "Created")}
				provenance={
					restored
						? t(
								"certificates.authority.restoredHere",
								"restored here in this session",
							)
						: undefined
				}
			>
				<DayOf at={view.createdAt} />
			</KvRow>
			<KvRow
				label={t("certificates.authority.signedFor", "Signed for")}
				provenance={t(
					"certificates.authority.signedForNote",
					"from live reads on this computer",
				)}
			>
				{signed.length ? (
					<span className="flex flex-col gap-0.5">
						{signed.map((row) => (
							<span key={row.key}>
								<DeviceLink
									deviceId={row.device.device_id}
									name={row.deviceName}
								/>
								{" · "}
								{certificateTitle(row)}
							</span>
						))}
					</span>
				) : (
					t("certificates.authority.signedForNone", "Nothing seen yet")
				)}
			</KvRow>
		</KeyValueList>
	);
}

function signingState(view: AuthorityView): ChainState {
	if (view.status !== "active") return "critical";
	return view.signingSoon ? "warning" : "good";
}

function signedState(signed: readonly CertificateRow[]): ChainState {
	if (!signed.length) return "unknown";
	return signed.some((row) => row.failing) ? "warning" : "good";
}

/** Root key (backup only) → signing key (this computer) → what it signed. */
function KeyChain({
	view,
	signed,
}: Readonly<Pick<FactsProps, "view" | "signed">>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const links: ChainLink[] = [
		{
			id: "root",
			icon: Archive,
			state: view.status === "root_expired" ? "critical" : "good",
			title: t("certificates.authority.chainRoot", "Root key"),
			text: t(
				"certificates.authority.chainRootText",
				"Only in your backup file. Not on this computer, never uploaded.",
			),
			join: t(
				"certificates.authority.chainRootJoin",
				"signs the signing key, valid until {{date}}",
				{ date: dayText(time, view.signingExpiresAt) },
			),
		},
		{
			id: "signing",
			icon: Laptop,
			state: signingState(view),
			title: t("certificates.authority.chainSigning", "Signing key"),
			text: t(
				"certificates.authority.chainSigningText",
				"On this computer, encrypted with the authority password",
			),
			join: t(
				"certificates.authority.chainSigningJoin",
				"signs requests from your devices, only for its allowed names",
			),
		},
		{
			id: "signed",
			icon: FileBadge,
			state: signedState(signed),
			title: t(
				"certificates.authority.chainSigned",
				"Renewal authorities and certificates",
			),
			text: signed.length
				? signed
						.slice(0, 3)
						.map((row) =>
							t(
								"certificates.authority.chainSignedItem",
								"{{certificate}} on {{device}}",
								{ certificate: certificateTitle(row), device: row.deviceName },
							),
						)
						.join(", ")
				: t(
						"certificates.authority.chainSignedNone",
						"None seen in live reads from this computer yet",
					),
		},
	];
	return (
		<TrustChain
			links={links}
			label={t(
				"certificates.authority.chainLabel",
				"Where the keys of {{label}} live",
				{ label: view.label },
			)}
		/>
	);
}

function ActionGroup({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<p className={LABEL}>{label}</p>
			<div className="flex flex-wrap gap-2">{children}</div>
		</div>
	);
}

interface ActionsProps {
	view: AuthorityView;
	onNote(note: AuthorityNote | null): void;
	onAction(action: AuthorityAction): void;
	/** The remove confirmation is open or running. */
	removing: boolean;
}

function AuthorityActions({
	view,
	onNote,
	onAction,
	removing,
}: Readonly<ActionsProps>) {
	const { t } = useTranslation("devices");
	const download = (kind: "root" | "signing") => {
		const file = `${shortId(view.id)}-${kind}.crt`;
		const bundle = view.authority.public_bundle;
		const root = kind === "root";
		saveFile(
			file,
			root ? bundle.root_certificate_pem : bundle.issuer_certificate_pem,
		);
		onNote({
			tone: "good",
			text: root
				? t(
						"certificates.authority.downloadedRoot",
						"{{file}} was handed to your browser. Install it on clients that should trust {{label}}, and compare its fingerprint with the one above.",
						{ file, label: view.label },
					)
				: t(
						"certificates.authority.downloadedSigning",
						"{{file}} was handed to your browser. Some clients need it next to the root to complete the chain.",
						{ file },
					),
		});
	};
	return (
		<div className="flex flex-wrap items-start gap-x-7 gap-y-3.5 border-t border-hairline pt-3.5">
			<ActionGroup
				label={t("certificates.authority.groupTrust", "Share trust")}
			>
				<DvButton size="sm" icon={Download} onClick={() => download("root")}>
					{t(
						"certificates.authority.downloadRoot",
						"Download root certificate",
					)}
				</DvButton>
				<DvButton size="sm" icon={Download} onClick={() => download("signing")}>
					{t(
						"certificates.authority.downloadSigning",
						"Download signing certificate",
					)}
				</DvButton>
			</ActionGroup>
			<ActionGroup
				label={t("certificates.authority.groupCare", "Look after it")}
			>
				<DvButton size="sm" icon={KeyRound} onClick={() => onAction("renew")}>
					{t("certificates.authority.renewKey", "Renew signing key…")}
				</DvButton>
				<DvButton size="sm" icon={Pencil} onClick={() => onAction("password")}>
					{t(
						"certificates.authority.changePassword",
						"Change authority password…",
					)}
				</DvButton>
				<DvButton size="sm" icon={Check} onClick={() => onAction("test")}>
					{t("certificates.authority.testPassword", "Test authority password…")}
				</DvButton>
				<DvButton size="sm" icon={Upload} onClick={() => onAction("restore")}>
					{t("certificates.authority.restore", "Restore from backup…")}
				</DvButton>
			</ActionGroup>
			<ActionGroup label={t("certificates.authority.groupRemove", "Remove")}>
				<DvButton
					size="sm"
					variant="danger-ghost"
					icon={Trash2}
					busy={removing}
					onClick={() => onAction("remove")}
				>
					{t(
						"certificates.authority.remove",
						"Remove signing key from this computer…",
					)}
				</DvButton>
			</ActionGroup>
		</div>
	);
}

function StorageFoot({ place }: Readonly<{ place: AuthorityPlace }>) {
	const { t } = useTranslation("devices");
	return (
		<span>
			{t(
				"certificates.authority.foot",
				"Stored on this computer for {{home}}. Other computers, browsers, hubs and profiles don't see it; restore the backup there to use it.",
				{ home: homeText(t, place.home) },
			)}
			{place.storage === "web_unprotected" ? (
				<span className="text-warning">
					{" "}
					<TriangleAlert aria-hidden className="inline size-3 align-[-1px]" />{" "}
					{t(
						"certificates.authority.webRisk",
						"This browser may delete site data, including this signing key. Keep the backup file safe.",
					)}
				</span>
			) : null}
			{place.storage === "web" ? (
				<>
					{" "}
					{t(
						"certificates.authority.web",
						"In a browser, clearing site data removes it; the backup file restores it.",
					)}
				</>
			) : null}
		</span>
	);
}

export interface AuthorityCardProps extends FactsProps, ActionsProps {
	place: AuthorityPlace;
	note?: AuthorityNote | null;
	/** Results of actions that run through the action layer. */
	results?: ReactNode;
}

/** SPEC §5.8: one organisation authority on this computer, its lifetime, where its keys live and what to do with it. */
export function AuthorityCard(props: Readonly<AuthorityCardProps>) {
	const { t } = useTranslation("devices");
	const { view, restored, signed, note, onNote, onAction } = props;
	return (
		<Block
			id={`authority-${shortId(view.id)}`}
			icon={ShieldCheck}
			title={view.label}
			summary={
				<span className="flex flex-wrap items-center gap-1.5">
					<StatusChips view={view} restored={restored} />
				</span>
			}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("certificates.authorities.stored", "stored on this computer")}
				/>
			}
			foot={<StorageFoot place={props.place} />}
			bodyClassName="gap-4"
		>
			<Warning view={view} onRenew={() => onAction("renew")} />
			<div className="grid grid-cols-[minmax(0,1.3fr)_minmax(300px,1fr)] items-start gap-x-8 gap-y-5 @max-[900px]/devices:grid-cols-1">
				<AuthorityFacts view={view} restored={restored} signed={signed} />
				<div className="flex min-w-0 flex-col gap-2.5 border-l border-hairline pl-6 @max-[900px]/devices:border-t @max-[900px]/devices:border-l-0 @max-[900px]/devices:pt-4 @max-[900px]/devices:pl-0">
					<p className={LABEL}>
						{t("certificates.authority.lifetime", "Lifetime")}
					</p>
					<Lifetime view={view} />
					<p className={cx(LABEL, "mt-2")}>
						{t("certificates.authority.keysLive", "Where its keys live")}
					</p>
					<KeyChain view={view} signed={signed} />
				</div>
			</div>
			<AuthorityActions
				view={view}
				onNote={onNote}
				onAction={onAction}
				removing={props.removing}
			/>
			{note ? (
				<InlineResult tone={note.tone} onDismiss={() => onNote(null)}>
					{note.text}
				</InlineResult>
			) : null}
			{props.results}
		</Block>
	);
}

/** An authority removed in this session: what it signed keeps working, and the backup brings it back. */
export function RemovedAuthorityCard({
	label,
	note,
	onDismissNote,
	onRestore,
}: Readonly<{
	label: string;
	note?: AuthorityNote | null;
	onDismissNote(): void;
	onRestore(): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			icon={ShieldOff}
			title={label}
			summary={
				<StatusChip tone="outline" icon={CircleDashed}>
					{t("certificates.authority.notHere", "Not on this computer")}
				</StatusChip>
			}
			stamp={
				<FreshnessStamp
					source="local"
					age="notloaded"
					text={t(
						"certificates.authority.removedStamp",
						"removed from this computer",
					)}
				/>
			}
		>
			{note ? (
				<InlineResult tone={note.tone} onDismiss={onDismissNote}>
					{note.text}
				</InlineResult>
			) : null}
			<p className={PROSE}>
				{t(
					"certificates.authority.removedText",
					"Certificates and renewal authorities it signed keep working. To sign with it again, restore it from its backup file with the authority password.",
				)}
			</p>
			<div>
				<DvButton size="sm" icon={Upload} onClick={onRestore}>
					{t("certificates.authority.restore", "Restore from backup…")}
				</DvButton>
			</div>
		</Block>
	);
}
