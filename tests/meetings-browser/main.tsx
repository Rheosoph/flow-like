import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { MeetingPage } from "../../packages/ui/components/meetings/meeting-page";
import { RoomMeetings } from "../../packages/ui/components/meetings/room-meetings";
import type {
	Meeting,
	MeetingClient,
	MeetingParticipant,
} from "../../packages/ui/lib/meetings/types";
import { useBackendStore } from "../../packages/ui/state/backend-state";
import "../../packages/ui/global.css";

const key = "meeting-browser-check";
const params = new URLSearchParams(location.search);
const host = params.get("view") === "host";
const schedule = params.get("view") === "schedule";
const now = Date.now();
const meeting: Meeting = {
	id: "design-review",
	conversation_id: "design-team",
	host_id: "host-account",
	title: "Design review: onboarding and customer feedback",
	starts_at: new Date(now - 60_000).toISOString(),
	ends_at: new Date(now + 3600_000).toISOString(),
	ended_at: null,
	guests_allowed: true,
};
const hostParticipant: MeetingParticipant = {
	id: "host-session",
	display_name: "Morgan Chen",
	guest: false,
	is_host: true,
	status: "admitted",
};
const guestParticipant: MeetingParticipant = {
	id: "guest-session",
	display_name: "Alex Rivera",
	guest: true,
	is_host: false,
	status: "waiting",
};
type State = {
	guests: MeetingParticipant[];
	ended: boolean;
	meetings: Meeting[];
	messages: {
		id: string;
		participant_id: string;
		display_name: string;
		body: string;
		created_at: string;
	}[];
};
function state(): State {
	return JSON.parse(
		localStorage.getItem(key) ??
			JSON.stringify({ guests: [], ended: false, meetings: [], messages: [] }),
	);
}
function save(value: State) {
	localStorage.setItem(key, JSON.stringify(value));
}
const client: MeetingClient = {
	async list() {
		return state().meetings;
	},
	async create(input) {
		const created = { ...meeting, ...input, id: `scheduled-${Date.now()}` };
		const current = state();
		current.meetings.push(created);
		save(current);
		return { meeting: created, invite_token: "fixture-invitation" };
	},
	async invite() {
		return { invite_token: "fixture-invitation" };
	},
	async rotateInvite() {
		return { invite_token: "replacement-invitation" };
	},
	async preview() {
		return meeting;
	},
	async join() {
		return { session_token: "host", participant_id: hostParticipant.id };
	},
	async joinGuest(_id, _token, name) {
		const current = state();
		current.guests = [{ ...guestParticipant, display_name: name }];
		save(current);
		return { session_token: "guest", participant_id: guestParticipant.id };
	},
	async session(_id, token) {
		const current = state();
		return {
			meeting: {
				...meeting,
				ended_at: current.ended ? new Date().toISOString() : null,
			},
			me:
				token === "host"
					? hostParticipant
					: (current.guests[0] ?? guestParticipant),
			participants: [hostParticipant, ...current.guests],
			messages: current.messages,
			server_time: new Date().toISOString(),
			room_epoch: 0,
		};
	},
	async realtime() {
		throw new Error("The fixture uses a local media room.");
	},
	async send(_id, token, body) {
		const current = state();
		const participant = token === "host" ? hostParticipant : current.guests[0];
		current.messages.push({
			id: String(Date.now()),
			participant_id: participant.id,
			display_name: participant.display_name,
			body,
			created_at: new Date().toISOString(),
		});
		save(current);
	},
	async leave() {},
	async admit(_id, participant) {
		const current = state();
		current.guests = current.guests.map((person) =>
			person.id === participant ? { ...person, status: "admitted" } : person,
		);
		save(current);
	},
	async remove(_id, participant) {
		const current = state();
		current.guests = current.guests.map((person) =>
			person.id === participant ? { ...person, status: "removed" } : person,
		);
		save(current);
	},
	async end() {
		const current = state();
		current.ended = true;
		save(current);
	},
};
useBackendStore.setState({
	backend: {
		userState: { lookupUsers: async () => [], lookupUser: async () => null },
		inboxState: {
			sendEntry: async () => {
				throw new Error("Meeting attempted to access a room inbox");
			},
		},
	} as never,
});
const queryClient = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
const container = document.getElementById("root");
if (!container) throw new Error("Fixture root is missing");
createRoot(container).render(
	<QueryClientProvider client={queryClient}>
		<div className="flex h-dvh min-h-0 flex-col">
			<div className="min-h-0 flex-1 overflow-auto" data-fixture-scroller>
				{schedule ? (
					<div className="p-8">
						<h1 className="mb-4 text-2xl">Design team</h1>
						<RoomMeetings
							conversationId="design-team"
							client={client}
							me="host-account"
							joinBaseUrl={`${location.origin}/meeting`}
						/>
					</div>
				) : (
					<MeetingPage
						meetingId={meeting.id}
						client={client}
						me={host ? "host-account" : undefined}
						displayName={host ? "Morgan Chen" : "Alex Rivera"}
						inviteToken={host ? undefined : "fixture-invitation"}
					/>
				)}
			</div>
		</div>
	</QueryClientProvider>,
);
