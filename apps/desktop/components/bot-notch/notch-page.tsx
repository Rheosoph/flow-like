"use client";

import dynamic from "next/dynamic";

const BotNotch = dynamic(
	() => import("./bot-notch").then((module) => module.BotNotch),
	{ ssr: false },
);

/** The notch window's page: transparent everywhere the goo and its cards are not. */
export function NotchPage() {
	return (
		<>
			<style>
				{"html,body{background:transparent!important;overflow:hidden}"}
			</style>
			<BotNotch />
		</>
	);
}
