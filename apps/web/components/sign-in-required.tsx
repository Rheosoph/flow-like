"use client";

import { useTranslation } from "@flow-like/locales";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import {
	ArrowRight,
	Cloud,
	Download,
	ExternalLink,
	Laptop,
	Puzzle,
	Sparkles,
	WifiOff,
	Workflow,
	Zap,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { currentRelativeUrl } from "../lib/return-url";

export function SignInRequired() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const [isRedirecting, setIsRedirecting] = useState(false);

	const handleSignIn = async () => {
		setIsRedirecting(true);
		try {
			// url_state rides the OAuth round trip itself, so the pre-login
			// location survives even where browser storage does not.
			await auth.signinRedirect({ url_state: currentRelativeUrl() });
		} catch (error) {
			console.error("Sign-in redirect failed:", error);
			setIsRedirecting(false);
		}
	};

	return (
		<div className="min-h-screen w-full bg-background relative overflow-auto">
			{/* Animated gradient background */}
			<div className="absolute inset-0 overflow-hidden pointer-events-none">
				{/* Mobile glow - subtle centered */}
				<div className="sm:hidden absolute top-1/4 left-1/2 -translate-x-1/2 w-[300px] h-[300px] bg-primary/10 rounded-full blur-[80px]" />
				{/* Desktop glows */}
				<div className="hidden sm:block absolute top-0 left-1/4 w-[600px] h-[600px] bg-primary/8 rounded-full blur-[120px] animate-pulse" />
				<div className="hidden sm:block absolute bottom-0 right-1/4 w-[500px] h-[500px] bg-violet-500/8 rounded-full blur-[100px] animate-pulse [animation-delay:1s]" />
				<div className="hidden sm:block absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[800px] h-[800px] bg-rose-500/5 rounded-full blur-[150px]" />
				{/* Grid pattern */}
				<div className="hidden sm:block absolute inset-0 bg-[linear-gradient(rgba(255,255,255,0.02)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,0.02)_1px,transparent_1px)] bg-[size:64px_64px]" />
			</div>

			<div className="relative z-10 min-h-screen flex flex-col items-center justify-center px-4 py-10 sm:py-12 lg:py-0 xl:py-0">
				{/* Logo with glow */}
				<div className="flex items-center gap-2 sm:gap-3 mb-6 sm:mb-10">
					<div className="relative">
						<div className="absolute inset-0 bg-primary/30 blur-xl rounded-full" />
						<Image
							src="/app-logo-light.webp"
							alt={t('flowlike2', 'Flow-Like')}
							width={56}
							height={56}
							className="dark:hidden relative w-10 h-10 sm:w-14 sm:h-14"
						/>
						<Image
							src="/app-logo.webp"
							alt={t('flowlike2', 'Flow-Like')}
							width={56}
							height={56}
							className="hidden dark:block relative w-10 h-10 sm:w-14 sm:h-14"
						/>
					</div>
					<span className="text-2xl sm:text-3xl font-bold tracking-tight bg-gradient-to-r from-foreground to-foreground/70 bg-clip-text">
						{t('flowlike2', 'Flow-Like')}
					</span>
				</div>

				{/* Main Card */}
				<div className="w-full max-w-md lg:max-w-5xl">
					<div className="rounded-3xl border border-border/50 bg-card/80 backdrop-blur-2xl shadow-2xl shadow-black/20 overflow-hidden lg:min-h-[620px]">
						<div className="flex flex-col lg:flex-row lg:min-h-[620px]">
							{/* Sign In Section */}
							<div className="p-7 sm:p-8 lg:px-12 lg:py-16 space-y-7 flex-1 flex flex-col lg:justify-center">
								{/* Header */}
								<div className="text-center space-y-3">
									<h1 className="text-3xl sm:text-4xl font-bold tracking-tight bg-gradient-to-br from-foreground via-foreground to-muted-foreground bg-clip-text">
										{t('welcomeBack', 'Welcome back')}
									</h1>
									<p className="text-muted-foreground text-sm sm:text-base">
										{t('signInToAccessYourAiWorkflows', 'Sign in to access your AI workflows')}
									</p>
								</div>

								{/* Sign In Button */}
								<div className="space-y-4">
									<Button
										className="w-full h-12 sm:h-14 text-sm sm:text-base font-semibold bg-gradient-to-r from-primary via-primary to-rose-500 hover:opacity-90 transition-all shadow-lg shadow-primary/25"
										size="lg"
										onClick={handleSignIn}
										disabled={isRedirecting}
									>
										{isRedirecting ? (
											<span className="flex items-center gap-2">
												<span className="h-5 w-5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
												Redirecting...
											</span>
										) : (
											<>
												{t('signInToContinue', 'Sign In to Continue')}
												<ArrowRight className="ml-2 h-5 w-5" />
											</>
										)}
									</Button>

									<p className="text-center text-xs sm:text-sm text-muted-foreground">
										{t('newHere', 'New here?')}{" "}
										<button
											type="button"
											onClick={handleSignIn}
											disabled={isRedirecting}
											className="text-primary font-semibold hover:underline underline-offset-2"
										>
											{t('createAFreeAccount', 'Create a free account')}
										</button>
									</p>
								</div>

								{/* Divider */}
								<div className="relative py-2">
									<div className="absolute inset-0 flex items-center">
										<div className="w-full border-t border-border/50" />
									</div>
									<div className="relative flex justify-center text-xs uppercase">
										<span className="bg-card px-4 text-muted-foreground font-medium tracking-wider">
											{t('orGetTheFullExperience', 'or get the full experience')}
										</span>
									</div>
								</div>

								{/* Mobile Studio Hint - visible only below lg */}
								<div className="lg:hidden space-y-3 pt-2">
									<div className="flex flex-col items-center text-center gap-1">
										<div className="flex items-center gap-2">
											<Sparkles className="h-4 w-4 text-primary" />
											<span className="font-semibold">{t('flowlikeStudio', 'Flow-Like Studio')}</span>
										</div>
										<span className="text-xs text-muted-foreground">
											{t('offlineAiCloudSync', 'Offline AI & Cloud Sync')}
										</span>
									</div>
									<div className="flex gap-2">
										<Button
											variant="outline"
											size="sm"
											className="flex-1 h-9 text-xs bg-background/60 hover:bg-background/80 border-border/50"
											asChild
										>
											<Link
												href="https://apps.apple.com/app/flow-like"
												target="_blank"
												rel="noopener noreferrer"
											>
												<svg
													className="mr-1 h-3.5 w-3.5"
													viewBox="0 0 24 24"
													fill="currentColor"
												>
													<path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
												</svg>
												{t('appStore', 'App Store')}
											</Link>
										</Button>
										<Button
											variant="outline"
											size="sm"
											className="flex-1 h-9 text-xs bg-background/60 hover:bg-background/80 border-border/50"
											asChild
										>
											<Link
												href="https://play.google.com/store/apps/details?id=com.flowlike.app"
												target="_blank"
												rel="noopener noreferrer"
											>
												<svg
													className="mr-1 h-3.5 w-3.5"
													viewBox="0 0 24 24"
													fill="currentColor"
												>
													<path d="M3.609 1.814L13.792 12 3.61 22.186a.996.996 0 01-.61-.92V2.734a1 1 0 01.609-.92zm10.89 10.893l2.302 2.302-10.937 6.333 8.635-8.635zm3.199-3.198l2.807 1.626a1 1 0 010 1.73l-2.808 1.626L15.206 12l2.492-2.491zM5.864 2.658L16.8 8.99l-2.302 2.302-8.634-8.634z" />
												</svg>
												{t('googlePlay', 'Google Play')}
											</Link>
										</Button>
									</div>
								</div>
							</div>

							{/* Studio Promo */}
							<div className="hidden lg:flex lg:flex-col lg:justify-center bg-gradient-to-br from-violet-600/15 via-primary/15 to-rose-500/15 border-t border-white/10 lg:border-t-0 lg:border-l lg:border-white/10 p-6 sm:p-7 lg:px-12 lg:py-16 space-y-5 relative overflow-hidden flex-1">
								{/* Animated background elements */}
								<div className="absolute -top-12 -right-12 w-40 h-40 bg-primary/20 rounded-full blur-3xl animate-pulse" />
								<div className="absolute -bottom-12 -left-12 w-40 h-40 bg-violet-500/20 rounded-full blur-3xl animate-pulse [animation-delay:0.5s]" />

								<div className="relative space-y-5">
									<div className="flex items-start justify-between">
										<div className="space-y-1.5">
											<div className="flex items-center gap-2.5">
												<div className="p-2 rounded-xl bg-gradient-to-br from-primary to-rose-500 shadow-lg shadow-primary/30">
													<Sparkles className="h-4 w-4 text-white" />
												</div>
												<span className="font-bold text-xl">
													{t('flowlikeStudio', 'Flow-Like Studio')}
												</span>
											</div>
											<p className="text-sm text-muted-foreground">
												{t('desktopMobileAppWithSuperpowers', 'Desktop & mobile app with superpowers')}
											</p>
										</div>
										<span className="text-xs font-semibold text-primary bg-primary/15 px-3 py-1.5 rounded-full border border-primary/25 shadow-sm">
											{t('recommended', 'Recommended')}
										</span>
									</div>

									<div className="grid grid-cols-3 gap-2.5">
										{[
											{ icon: Cloud, label: t('cloudSync', 'Cloud Sync') },
											{ icon: WifiOff, label: t('offline', 'Offline') },
											{ icon: Laptop, label: t('localAi', 'Local AI') },
										].map((item) => (
											<div
												key={item.label}
												className="flex flex-col items-center gap-2 p-3 rounded-xl bg-background/60 border border-border/40 text-center backdrop-blur-sm hover:bg-background/80 transition-colors"
											>
												<div className="p-2.5 rounded-xl bg-muted/80">
													<item.icon className="h-4 w-4 text-foreground/70" />
												</div>
												<span className="text-xs font-medium">
													{item.label}
												</span>
											</div>
										))}
									</div>

									<Button
										className="w-full h-11 sm:h-12 text-sm sm:text-base font-semibold bg-gradient-to-r from-primary via-rose-500 to-primary bg-[length:200%_100%] hover:bg-[position:100%_0] transition-all duration-500 shadow-lg shadow-primary/20"
										asChild
									>
										<Link
											href="https://flow-like.com/download"
											target="_blank"
											rel="noopener noreferrer"
										>
											<Download className="mr-2 h-4 w-4" />
											{t('downloadForFree', 'Download for Free')}
											<ExternalLink className="ml-2 h-3.5 w-3.5 opacity-70" />
										</Link>
									</Button>

									<div className="flex flex-col sm:flex-row gap-2.5">
										<Button
											variant="outline"
											size="sm"
											className="flex-1 h-10 bg-background/60 hover:bg-background/80 border-border/50"
											asChild
										>
											<Link
												href="https://apps.apple.com/app/flow-like"
												target="_blank"
												rel="noopener noreferrer"
											>
												<svg
													className="mr-1.5 h-4 w-4"
													viewBox="0 0 24 24"
													fill="currentColor"
												>
													<path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
												</svg>
												{t('appStore', 'App Store')}
											</Link>
										</Button>
										<Button
											variant="outline"
											size="sm"
											className="flex-1 h-10 bg-background/60 hover:bg-background/80 border-border/50"
											asChild
										>
											<Link
												href="https://play.google.com/store/apps/details?id=com.flowlike.app"
												target="_blank"
												rel="noopener noreferrer"
											>
												<svg
													className="mr-1.5 h-4 w-4"
													viewBox="0 0 24 24"
													fill="currentColor"
												>
													<path d="M3.609 1.814L13.792 12 3.61 22.186a.996.996 0 01-.61-.92V2.734a1 1 0 01.609-.92zm10.89 10.893l2.302 2.302-10.937 6.333 8.635-8.635zm3.199-3.198l2.807 1.626a1 1 0 010 1.73l-2.808 1.626L15.206 12l2.492-2.491zM5.864 2.658L16.8 8.99l-2.302 2.302-8.634-8.634z" />
												</svg>
												{t('googlePlay', 'Google Play')}
											</Link>
										</Button>
									</div>
								</div>
							</div>
						</div>
					</div>

					{/* Footer features - Updated stats */}
					<div className="hidden sm:flex flex-wrap justify-center gap-6 mt-10 text-muted-foreground">
						{[
							{ icon: Zap, label: t('nocodeAi', 'No-Code AI'), color: "text-yellow-500" },
							{ icon: Workflow, label: t('500Nodes', '500+ Nodes'), color: "text-primary" },
							{
								icon: Puzzle,
								label: t('100Integrations', '100+ Integrations'),
								color: "text-violet-500",
							},
						].map((item) => (
							<div
								key={item.label}
								className="flex items-center gap-2 text-sm font-medium"
							>
								<item.icon className={`h-4 w-4 ${item.color}`} />
								<span>{item.label}</span>
							</div>
						))}
					</div>
				</div>
			</div>
		</div>
	);
}
