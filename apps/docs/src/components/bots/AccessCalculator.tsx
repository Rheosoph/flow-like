import {
	RESTRICTION_PRESETS,
	holds,
	presetRestrictions,
} from "@flow-like/flow-like-ui/components/bots-tab/bot-restrictions-model";
import { useState } from "react";
import {
	type BotPermission,
	type BotRestrictionPreset,
	permissionLabel,
	restrictionPresetLabel,
} from "./bot-docs-data";
import { ToneIcon } from "./ui";
import "./access-calculator.css";

type RoomLimit = "none" | BotRestrictionPreset;

const PERMISSIONS: readonly BotPermission[] = [
	"read_files",
	"write_files",
	"read_database",
	"write_database",
	"read_boards",
	"execute_boards",
	"write_boards",
	"read_events",
	"execute_events",
	"write_events",
];

const DEFAULT_ROLE: readonly BotPermission[] = PERMISSIONS.filter(
	(id) => id !== "write_boards" && id !== "write_events",
);

function Mark({ on }: { on: boolean }) {
	return (
		<span className="bd-access-mark" data-on={on}>
			<ToneIcon tone={on ? "good" : "bad"} />
			<span className="sr-only">{on ? "Allowed" : "Not allowed"}</span>
		</span>
	);
}

export default function AccessCalculator() {
	const [role, setRole] = useState<readonly BotPermission[]>(DEFAULT_ROLE);
	const [botPreset, setBotPreset] = useState<BotRestrictionPreset>(
		"read_and_call_events",
	);
	const [roomLimit, setRoomLimit] = useState<RoomLimit>("read_only");

	const bot = presetRestrictions(botPreset);
	const room = roomLimit === "none" ? null : presetRestrictions(roomLimit);
	const allowed = PERMISSIONS.filter(
		(id) => role.includes(id) && holds(bot, id) && (!room || holds(room, id)),
	);

	const toggleRole = (id: BotPermission) =>
		setRole(
			role.includes(id) ? role.filter((item) => item !== id) : [...role, id],
		);

	return (
		<section
			className="bd not-content"
			aria-label="What a bot may do in an app"
		>
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Access calculator</span>
					<p className="bd-title">
						Your role ∩ the bot's limits ∩ the room's limits
					</p>
					<p className="bd-sub">
						A bot never gets more than you have. Tick what your role allows,
						then narrow it for the bot and the room.
					</p>
				</div>
			</div>
			<div className="bd-access-controls">
				<label className="bd-field">
					<span className="bd-label">What the bot may do in this app</span>
					<select
						className="bd-select"
						value={botPreset}
						onChange={(event) =>
							setBotPreset(event.target.value as BotRestrictionPreset)
						}
					>
						{RESTRICTION_PRESETS.map((preset) => (
							<option key={preset} value={preset}>
								{restrictionPresetLabel(preset)}
							</option>
						))}
					</select>
				</label>
				<label className="bd-field">
					<span className="bd-label">Room limit</span>
					<select
						className="bd-select"
						value={roomLimit}
						onChange={(event) => setRoomLimit(event.target.value as RoomLimit)}
					>
						<option value="none">Your 1:1 chat (no room limit)</option>
						{RESTRICTION_PRESETS.map((preset) => (
							<option key={preset} value={preset}>
								Room: {restrictionPresetLabel(preset)}
							</option>
						))}
					</select>
				</label>
			</div>
			<div className="bd-access-table">
				<table aria-label="Effective permissions">
					<thead>
						<tr>
							<th scope="col">Permission</th>
							<th scope="col">Your role</th>
							<th scope="col">Bot</th>
							<th scope="col">Room</th>
							<th scope="col">Result</th>
						</tr>
					</thead>
					<tbody>
						{PERMISSIONS.map((id) => {
							const result = allowed.includes(id);
							return (
								<tr key={id} data-result={result}>
									<th scope="row">{permissionLabel(id)}</th>
									<td>
										<input
											type="checkbox"
											aria-label={`Your role allows: ${permissionLabel(id)}`}
											checked={role.includes(id)}
											onChange={() => toggleRole(id)}
										/>
									</td>
									<td>
										<Mark on={holds(bot, id)} />
									</td>
									<td>
										{room ? (
											<Mark on={holds(room, id)} />
										) : (
											<span className="bd-access-na">—</span>
										)}
									</td>
									<td>
										<Mark on={result} />
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			</div>
			<p className="bd-note">
				“Full (your role)” passes your role through unchanged. Some rights are
				never given to a bot, whatever your role: owner and admin rights,
				reading roles, editing the app's details, settings or routes, and
				reading analytics.
			</p>
		</section>
	);
}
