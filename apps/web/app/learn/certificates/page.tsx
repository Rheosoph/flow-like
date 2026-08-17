"use client";
import { useTranslation } from "@flow-like/locales";
import {
	CertificateCard,
	EmptyState,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useQuery } from "@tanstack/react-query";
import { motion } from "framer-motion";
import { Award, Compass, ScrollText, Sparkles } from "lucide-react";
import { useRouter } from "next/navigation";
import { useAuth } from "react-oidc-context";
import { learnApi } from "../../../lib/learn-api";

export default function CertificatesPage() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const router = useRouter();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";

	const certificatesQuery = useQuery({
		queryKey: [
			"learn",
			"certificates",
			"me",
			profileId,
			auth.user?.profile.sub,
		],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.myCertificates(profile!, auth),
	});

	const certificates = certificatesQuery.data ?? [];

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-5xl p-6 md:p-10 space-y-8">
				{/* Hero */}
				<motion.section
					initial={{ opacity: 0, y: 12 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.45, ease: "easeOut" }}
					className="text-center space-y-3 pt-4"
				>
					<div className="inline-flex items-center justify-center size-14 rounded-2xl bg-linear-to-br from-amber-400/20 to-orange-600/10 ring-1 ring-amber-500/30">
						<Award className="size-7 text-amber-500" />
					</div>
					<h1 className="text-3xl md:text-4xl font-semibold tracking-tight">
						{t('yourCertificates', 'Your certificates')}
					</h1>
					<p className="text-muted-foreground">
						{t('eachCertificateIsSignedAndVerifiableByHash', 'Each certificate is signed and verifiable by hash.')}
					</p>
				</motion.section>

				{certificates.length === 0 ? (
					<div className="flex justify-center py-6">
						<EmptyState
							title={t('noCertificatesYet', 'No certificates — yet')}
							description={
								t('finishACourseEndtoendToEarnOneAndPutItOnDisplay', "Finish a course end-to-end to earn one and put it on display.")
							}
							icons={[Compass, ScrollText, Sparkles]}
							action={{
								label: "Browse courses",
								onClick: () => router.push("/learn"),
							}}
						/>
					</div>
				) : (
					<div className="grid gap-5 grid-cols-1 md:grid-cols-2">
						{certificates.map((c, i) => (
							<CertificateCard
								key={c.id}
								certificate={c}
								verifyUrl={(id) => `/learn/certificates/verify/${id}`}
								index={i}
							/>
						))}
					</div>
				)}
			</div>
		</div>
	);
}
