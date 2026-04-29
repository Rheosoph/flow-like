"use client";
import { motion } from "framer-motion";
import {
	Award,
	Copy,
	ExternalLink,
	ScrollText,
	ShieldCheck,
	Sparkles,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useMemo } from "react";
import { toast } from "sonner";
import type { CertificateView } from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";

interface CertificateCardProps {
	readonly certificate: CertificateView;
	readonly verifyUrl: (certId: string) => string;
	readonly index?: number;
}

export function CertificateCard({
	certificate,
	verifyUrl,
	index = 0,
}: CertificateCardProps) {
	const issued = new Date(certificate.issued_at).toLocaleDateString(undefined, {
		dateStyle: "long",
	});
	const shortHash = certificate.hash.slice(0, 8);
	const verifyLink = verifyUrl(certificate.id);
	const recipient = certificate.recipient_name?.trim() || "Anonymous learner";
	const courseTitle =
		certificate.course_name?.trim() || `Course ${certificate.course_id}`;

	const qrPayload = useMemo(
		() =>
			JSON.stringify({
				url: verifyLink,
				id: certificate.id,
				hash: certificate.hash,
			}),
		[verifyLink, certificate.id, certificate.hash],
	);

	return (
		<motion.article
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			transition={{ duration: 0.4, delay: index * 0.05, ease: "easeOut" }}
			className="relative overflow-hidden rounded-2xl border border-border/50 bg-linear-to-br from-amber-50/40 via-card to-orange-50/30 dark:from-amber-950/30 dark:via-card dark:to-orange-950/20 backdrop-blur-sm shadow-sm hover:shadow-xl hover:border-amber-500/40 hover:-translate-y-0.5 transition-[box-shadow,border-color,transform] duration-200 ease-out"
		>
			{/* corner ornaments */}
			<CornerOrnament className="top-2 left-2" />
			<CornerOrnament className="top-2 right-2 rotate-90" />
			<CornerOrnament className="bottom-2 left-2 -rotate-90" />
			<CornerOrnament className="bottom-2 right-2 rotate-180" />

			<div className="relative px-6 pt-7 pb-6 grid grid-cols-[1fr_auto] gap-5 items-start">
				{/* left: text content */}
				<div className="space-y-4 min-w-0">
					<div className="flex items-center gap-2 text-[10px] uppercase tracking-[0.2em] font-semibold text-amber-700 dark:text-amber-300/90">
						<ScrollText className="size-3" />
						Certificate of completion
					</div>

					<div className="space-y-1">
						<div className="text-xs uppercase tracking-wide text-muted-foreground/80">
							Awarded to
						</div>
						<div className="text-xl md:text-2xl font-serif font-semibold leading-tight text-foreground">
							{recipient}
						</div>
					</div>

					<div className="space-y-1">
						<div className="text-xs uppercase tracking-wide text-muted-foreground/80">
							For completing
						</div>
						<div className="text-base font-medium leading-snug line-clamp-2">
							{courseTitle}
						</div>
					</div>

					<div className="flex items-center gap-3 text-xs text-muted-foreground pt-1">
						<span className="inline-flex items-center gap-1">
							<Award className="size-3 text-amber-500" />
							Issued {issued}
						</span>
						<span aria-hidden>·</span>
						<span className="inline-flex items-center gap-1 font-mono">
							<ShieldCheck className="size-3" />
							{shortHash}
						</span>
					</div>
				</div>

				{/* right: QR code */}
				<div className="shrink-0">
					<a
						href={verifyLink}
						className="block rounded-xl bg-white p-2.5 shadow-md ring-1 ring-amber-500/30 hover:ring-amber-500/60 transition-[box-shadow,outline-color,filter] duration-200"
						title="Open verification page"
					>
						<QRCodeSVG
							value={qrPayload}
							size={104}
							level="M"
							marginSize={0}
							bgColor="#ffffff"
							fgColor="#1a1a1a"
						/>
					</a>
					<div className="mt-1.5 text-center text-[9px] uppercase tracking-wider text-muted-foreground">
						Scan to verify
					</div>
				</div>
			</div>

			<div className="relative px-6 pb-5 pt-2 border-t border-border/40 bg-background/30 backdrop-blur-sm flex flex-wrap gap-2 items-center">
				<Sparkles className="size-3 text-amber-500" />
				<span className="text-[10px] uppercase tracking-wide text-muted-foreground font-medium">
					Verifiable on-chain via
				</span>
				<code className="text-[10px] truncate max-w-45 text-muted-foreground/80">
					{verifyLink}
				</code>

				<div className="ml-auto flex flex-wrap gap-1.5">
					<Button
						size="sm"
						variant="ghost"
						className="rounded-lg gap-1.5 h-7 text-xs"
						onClick={async () => {
							try {
								await navigator.clipboard.writeText(certificate.hash);
								toast.success("Hash copied to clipboard");
							} catch {
								toast.error("Could not access clipboard");
							}
						}}
					>
						<Copy className="size-3" />
						Copy hash
					</Button>
					<Button
						asChild
						size="sm"
						variant="outline"
						className="rounded-lg gap-1.5 h-7 text-xs"
					>
						<a href={verifyLink}>
							<ExternalLink className="size-3" />
							Verify
						</a>
					</Button>
					{certificate.pdf_url && (
						<Button
							asChild
							size="sm"
							className={cn("rounded-lg gap-1.5 h-7 text-xs")}
						>
							<a
								href={certificate.pdf_url}
								target="_blank"
								rel="noreferrer"
							>
								<ScrollText className="size-3" />
								PDF
							</a>
						</Button>
					)}
				</div>
			</div>
		</motion.article>
	);
}

function CornerOrnament({ className }: { readonly className?: string }) {
	return (
		<svg
			viewBox="0 0 28 28"
			width="28"
			height="28"
			aria-hidden
			className={cn(
				"absolute pointer-events-none text-amber-500/40",
				className,
			)}
		>
			<path
				d="M2 14 L2 2 L14 2"
				stroke="currentColor"
				strokeWidth="1"
				fill="none"
			/>
			<circle cx="2" cy="2" r="1.5" fill="currentColor" />
		</svg>
	);
}
