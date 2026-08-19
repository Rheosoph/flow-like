"use client";
import { Trans, useTranslation } from "@flow-like/locales";
import {
	Button,
	type CertificateView,
	Input,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useQuery } from "@tanstack/react-query";
import { motion } from "framer-motion";
import {
	ArrowLeft,
	Award,
	CheckCircle2,
	Copy,
	ScrollText,
	ShieldCheck,
	Sparkles,
	XCircle,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { learnApi } from "../../lib/learn-api";

export default function VerifyPage() {
	const { t } = useTranslation("common");
	const params = useSearchParams();
	const certId = params.get("id") ?? "";
	const auth = useAuth();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;

	const [origin, setOrigin] = useState("");
	useEffect(() => {
		if (typeof window !== "undefined") setOrigin(window.location.origin);
	}, []);

	const certQuery = useQuery({
		queryKey: ["verify", "certificate", certId],
		enabled: Boolean(profile && certId),
		queryFn: () => learnApi.verifyCertificate(profile!, auth, certId),
		retry: false,
	});

	if (!certId) {
		return <VerifyLookupScreen />;
	}

	if (certQuery.isPending) {
		return <LoadingScreen />;
	}

	if (certQuery.isError || !certQuery.data) {
		const reason =
			certQuery.error instanceof Error
				? certQuery.error.message
				: t('thisCertificateDoesntExistOrHasBeenRevoked', 'This certificate doesn\'t exist or has been revoked.');
		return (
			<MissingCertScreen
				title={t('certificateNotFound', 'Certificate not found')}
				description={reason}
				certId={certId}
			/>
		);
	}

	const cert = certQuery.data;
	return <VerifiedScreen cert={cert} origin={origin} />;
}

function VerifiedScreen({
	cert,
	origin,
}: {
	readonly cert: CertificateView;
	readonly origin: string;
}) {
	const { t } = useTranslation("common");
	const issued = new Date(cert.issued_at).toLocaleDateString(undefined, {
		dateStyle: "long",
	});
	const recipient = cert.recipient_name?.trim() || t('anonymousLearner', 'Anonymous learner');
	const courseTitle = cert.course_name?.trim() || t('courseCourse_id', 'Course {{course_id}}', { course_id: cert.course_id });
	const verifyLink = `${origin}/verify?id=${cert.id}`;
	const qrPayload = useMemo(
		() =>
			JSON.stringify({
				url: verifyLink,
				id: cert.id,
				hash: cert.hash,
			}),
		[verifyLink, cert.id, cert.hash],
	);

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-3xl p-6 md:p-12 space-y-8">
				<Link
					href="/learn"
					className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
				>
					<ArrowLeft className="h-3 w-3" />
					{t('flowlikeUniversity', 'Flow-Like University')}
				</Link>

				{/* Verified badge */}
				<motion.div
					initial={{ opacity: 0, scale: 0.95 }}
					animate={{ opacity: 1, scale: 1 }}
					transition={{ duration: 0.4, ease: "easeOut" }}
					className="flex items-center gap-2 self-start rounded-full bg-emerald-500/10 ring-1 ring-emerald-500/30 text-emerald-700 dark:text-emerald-400 px-4 py-1.5 text-sm font-medium w-fit"
				>
					<CheckCircle2 className="size-4" />
					{t('verifiedCertificate', 'Verified certificate')}
				</motion.div>

				{/* The certificate */}
				<motion.article
					initial={{ opacity: 0, y: 12 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.5, delay: 0.1, ease: "easeOut" }}
					className="relative overflow-hidden rounded-3xl border border-amber-500/30 bg-linear-to-br from-amber-50/60 via-card to-orange-50/40 dark:from-amber-950/40 dark:via-card dark:to-orange-950/30 backdrop-blur-sm p-8 md:p-12 shadow-2xl"
				>
					<CornerOrnament className="top-4 left-4" />
					<CornerOrnament className="top-4 right-4 rotate-90" />
					<CornerOrnament className="bottom-4 left-4 -rotate-90" />
					<CornerOrnament className="bottom-4 right-4 rotate-180" />

					<div className="relative grid md:grid-cols-[1fr_auto] gap-8 items-start">
						<div className="space-y-6">
							<div className="flex items-center gap-2 text-[11px] uppercase tracking-[0.2em] font-semibold text-amber-700 dark:text-amber-300">
								<ScrollText className="size-3.5" />
								{t('certificateOfCompletion', 'Certificate of completion')}
							</div>

							<div className="space-y-1.5">
								<div className="text-xs uppercase tracking-wide text-muted-foreground">
									{t('thisCertifiesThat', 'This certifies that')}
								</div>
								<div className="text-3xl md:text-4xl font-serif font-semibold leading-tight text-foreground">
									{recipient}
								</div>
							</div>

							<div className="space-y-1.5">
								<div className="text-xs uppercase tracking-wide text-muted-foreground">
									{t('hasSuccessfullyCompleted', 'has successfully completed')}
								</div>
								<div className="text-xl md:text-2xl font-medium leading-snug">
									{courseTitle}
								</div>
							</div>

							<div className="flex items-center gap-4 pt-2">
								<div>
									<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
										{t('issued', 'Issued')}
									</div>
									<div className="text-sm font-medium">{issued}</div>
								</div>
								<div className="size-px h-8 bg-border self-end" />
								<div>
									<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
										{t('certificateId', 'Certificate ID')}
									</div>
									<div className="text-sm font-mono">{cert.id}</div>
								</div>
							</div>
						</div>

						<div className="shrink-0">
							<div className="rounded-2xl bg-white p-3 shadow-lg ring-2 ring-amber-500/40">
								<QRCodeSVG
									value={qrPayload}
									size={144}
									level="M"
									marginSize={0}
									bgColor="#ffffff"
									fgColor="#1a1a1a"
								/>
							</div>
							<div className="mt-2 text-center text-[10px] uppercase tracking-wider text-muted-foreground">
								{t('scanToReverify', 'Scan to re-verify')}
							</div>
						</div>
					</div>

					<div className="relative mt-8 pt-6 border-t border-border/50 flex flex-wrap items-center gap-3">
						<div className="flex items-center gap-2">
							<div className="size-9 rounded-full bg-linear-to-br from-amber-400 to-orange-500 grid place-items-center shadow-md ring-2 ring-background">
								<Award className="size-4 text-amber-50" />
							</div>
							<div>
								<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
									{t('issuedBy', 'Issued by')}
								</div>
								<div className="text-sm font-semibold">
									{t('flowlikeUniversity', 'Flow-Like University')}
								</div>
							</div>
						</div>

						<div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
							<Sparkles className="size-3 text-amber-500" />
							<span>{t('sha256Verified', 'SHA-256 verified')}</span>
						</div>
					</div>
				</motion.article>

				{/* Hash detail */}
				<motion.div
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.45, delay: 0.2, ease: "easeOut" }}
					className="rounded-xl border border-border/50 bg-card/60 backdrop-blur-sm p-4 md:p-5 space-y-3"
				>
					<div className="flex items-center gap-2 text-sm font-medium">
						<ShieldCheck className="size-4 text-emerald-500" />
						{t('verificationHash', 'Verification hash')}
					</div>
					<div className="rounded-lg bg-muted/50 px-3 py-2 font-mono text-xs break-all">
						<span className="text-amber-600 dark:text-amber-400 font-semibold">
							{cert.hash.slice(0, 8)}
						</span>
						<span>{cert.hash.slice(8)}</span>
					</div>
					<div className="flex flex-wrap gap-2">
						<Button
							size="sm"
							variant="outline"
							onClick={() => {
								void navigator.clipboard.writeText(cert.hash);
							}}
						>
							<Copy className="size-3.5 mr-1.5" />
							{t('copyHash', 'Copy hash')}
						</Button>
						<Button
							size="sm"
							variant="ghost"
							onClick={() => {
								void navigator.clipboard.writeText(verifyLink);
							}}
						>
							<Copy className="size-3.5 mr-1.5" />
							{t('copyVerifyLink', 'Copy verify link')}
						</Button>
					</div>
				</motion.div>
			</div>
		</div>
	);
}

function CornerOrnament({ className }: { readonly className?: string }) {
	return (
		<svg
			viewBox="0 0 36 36"
			width="36"
			height="36"
			aria-hidden
			className={`absolute pointer-events-none text-amber-500/50 ${className ?? ""}`}
		>
			<path
				d="M2 18 L2 2 L18 2"
				stroke="currentColor"
				strokeWidth="1.5"
				fill="none"
			/>
			<circle cx="2" cy="2" r="2" fill="currentColor" />
		</svg>
	);
}

function LoadingScreen() {
	return (
		<div className="flex-1 grid place-items-center p-12">
			<div className="flex items-center gap-2 text-muted-foreground"><Trans i18nKey="divClassnamesize4RoundedfullBorder2Bordermutedforeground30BordertforegroundAnimatespinVerifyingCertificate"><div className="size-4 rounded-full border-2 border-muted-foreground/30 border-t-foreground animate-spin" />
				Verifying certificate…</Trans></div>
		</div>
	);
}

function MissingCertScreen({
	title,
	description,
	certId,
}: {
	readonly title: string;
	readonly description: string;
	readonly certId?: string;
}) {
	const { t } = useTranslation("common");
	return (
		<div className="flex-1 grid place-items-center p-6">
			<div className="text-center space-y-4 max-w-md">
				<div className="inline-flex items-center justify-center size-14 rounded-2xl bg-rose-500/10 ring-1 ring-rose-500/30">
					<XCircle className="size-7 text-rose-500" />
				</div>
				<h1 className="text-2xl font-semibold">{title}</h1>
				<p className="text-muted-foreground">{description}</p>
				{certId && (
					<div className="rounded-lg border border-border/40 bg-card/40 px-3 py-2 text-left space-y-1">
						<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
							{t('lookedUp', 'Looked up')}
						</div>
						<code className="text-xs font-mono break-all">{certId}</code>
					</div>
				)}
				<div className="flex items-center justify-center gap-3 pt-2">
					<Link
						href="/verify"
						className="inline-flex items-center gap-1 text-sm text-primary hover:underline"
					>
						{t('tryAnotherId', 'Try another ID')}
					</Link>
					<span className="text-xs text-muted-foreground">·</span>
					<Link
						href="/learn"
						className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
					>
						<ArrowLeft className="size-3" />
						{t('flowlikeUniversity', 'Flow-Like University')}
					</Link>
				</div>
			</div>
		</div>
	);
}

function VerifyLookupScreen() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const [input, setInput] = useState("");

	const trimmed = input.trim();
	const extractedId = useMemo(() => extractCertificateId(trimmed), [trimmed]);

	const onSubmit = (e: React.FormEvent) => {
		e.preventDefault();
		if (!extractedId) return;
		router.push(`/verify?id=${encodeURIComponent(extractedId)}`);
	};

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-xl p-6 md:p-12 space-y-6">
				<Link
					href="/learn"
					className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
				>
					<ArrowLeft className="h-3 w-3" />
					{t('flowlikeUniversity', 'Flow-Like University')}
				</Link>

				<motion.div
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.4, ease: "easeOut" }}
					className="space-y-3 text-center"
				>
					<div className="inline-flex items-center justify-center size-14 rounded-2xl bg-linear-to-br from-emerald-400/20 to-sky-500/10 ring-1 ring-emerald-500/30">
						<ShieldCheck className="size-7 text-emerald-500" />
					</div>
					<h1 className="text-3xl font-semibold tracking-tight">
						{t('verifyACertificate', 'Verify a certificate')}
					</h1>
					<p className="text-muted-foreground">
						{t('pasteACertificateIdSha256HashOrVerifyLinkToConfirmAuthenticity', "Paste a certificate ID, SHA-256 hash, or verify link to confirm authenticity.")}
					</p>
				</motion.div>

				<motion.form
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.45, delay: 0.1, ease: "easeOut" }}
					onSubmit={onSubmit}
					className="rounded-2xl border border-border/50 bg-card/60 backdrop-blur-sm p-5 space-y-4"
				>
					<div className="space-y-2">
						<label
							htmlFor="cert-lookup-input"
							className="text-xs uppercase tracking-wide text-muted-foreground font-medium"
						>
							{t('certificateIdHashOrVerifyUrl', 'Certificate ID, hash, or verify URL')}
						</label>
						<Input
							id="cert-lookup-input"
							autoFocus
							value={input}
							onChange={(e) => setInput(e.target.value)}
							placeholder="gf37n7m6fqxws611qfopdcef"
							className="font-mono"
						/>
						{trimmed && !extractedId && (
							<p className="text-xs text-rose-500">
								{t('doesntLookLikeACertificateIdOrVerifyLink', 'Doesn\'t look like a certificate ID or verify link.')}
							</p>
						)}
					</div>
					<div className="flex items-center justify-between gap-2">
						<p className="text-xs text-muted-foreground">
							{t('tipScanTheQrCodeOnACertificateToJumpStraightIn', 'Tip: scan the QR code on a certificate to jump straight in.')}
						</p>
						<Button type="submit" disabled={!extractedId} className="gap-1.5">
							<ShieldCheck className="size-4" />
							{t('verify', 'Verify')}
						</Button>
					</div>
				</motion.form>
			</div>
		</div>
	);
}

function extractCertificateId(input: string): string | null {
	if (!input) return null;
	try {
		const url = new URL(input);
		const queryId = url.searchParams.get("id");
		if (queryId) return queryId;
	} catch {
		// not a URL — fall through to id check
	}
	if (/^[a-z0-9_-]{8,64}$/i.test(input)) return input;
	return null;
}
