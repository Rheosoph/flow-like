import { useId, useReducer, useState } from "react";
import {
	channelTransition,
	emptyCache,
	emptyChannel,
	exampleFeed,
	runCache,
	selectFeed,
} from "./community-explainer-models";
import "./community-explainer.css";

function FeedExplainer() {
	const id = useId();
	const [query, setQuery] = useState("");
	const [category, setCategory] = useState("");
	const [limit, setLimit] = useState(3);
	const entries = selectFeed(exampleFeed, query, category, limit);
	return (
		<>
			<h3>Build your reading list</h3>
			<p>
				Filter five invented feed entries. The two copies of the database
				release become one item, then the newest dated items come first.
			</p>
			<div className="ce-controls">
				<label htmlFor={`${id}-query`}>
					Title contains
					<input
						id={`${id}-query`}
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder="Try research"
					/>
				</label>
				<label htmlFor={`${id}-category`}>
					Category
					<select
						id={`${id}-category`}
						value={category}
						onChange={(event) => setCategory(event.target.value)}
					>
						<option value="">All categories</option>
						<option>Engineering</option>
						<option>Research</option>
					</select>
				</label>
				<label htmlFor={`${id}-limit`}>
					Maximum entries
					<select
						id={`${id}-limit`}
						value={limit}
						onChange={(event) => setLimit(Number(event.target.value))}
					>
						<option value={1}>1</option>
						<option value={3}>3</option>
						<option value={5}>5</option>
					</select>
				</label>
			</div>
			<div className="ce-output" aria-live="polite" aria-atomic="true">
				<p className="ce-summary">
					{entries.length} matching {entries.length === 1 ? "entry" : "entries"}
				</p>
				{entries.length ? (
					<ol>
						{entries.map((entry) => (
							<li key={entry.id}>
								<strong>{entry.title}</strong>
								<span>
									{entry.date ?? "No publication date"} · {entry.category} ·{" "}
									{entry.format}
								</span>
							</li>
						))}
					</ol>
				) : (
					<p>No titles match. Clear the text filter or change the category.</p>
				)}
			</div>
			<details>
				<summary>See the input entries</summary>
				<ul>
					{exampleFeed.map((entry, index) => (
						<li key={`${entry.id}-${index}`}>
							{entry.title} · {entry.format} · {entry.date ?? "undated"}
						</li>
					))}
				</ul>
			</details>
			<p className="ce-note">
				This illustration filters titles and uses an example ID for
				deduplication. RSS Utils searches additional fields and has its own item
				identity rules. Array Sort supplies ordering in the actual workflow.
			</p>
		</>
	);
}

function CacheExplainer() {
	const id = useId();
	const [state, setState] = useState(emptyCache);
	const [version, setVersion] = useState("v1");
	return (
		<>
			<h3>Follow the cache branch</h3>
			<p>
				Run the same request twice. Then change the source version or advance
				the example clock to see why the workflow computes again.
			</p>
			<label htmlFor={`${id}-version`}>
				Source version
				<select
					id={`${id}-version`}
					value={version}
					onChange={(event) => setVersion(event.target.value)}
				>
					<option>v1</option>
					<option>v2</option>
				</select>
			</label>
			<p className="ce-key">
				Key: <code>example-team:handbook:{version}</code>
			</p>
			<div className="ce-actions">
				<button
					type="button"
					onClick={() => setState((current) => runCache(current, version))}
				>
					Run request
				</button>
				<button
					type="button"
					className="ce-secondary"
					onClick={() =>
						setState((current) => ({
							...current,
							clock: current.clock + 1,
							last: "The example clock advanced one hour. Run a request to check expiry.",
						}))
					}
				>
					Advance one hour
				</button>
				<button
					type="button"
					className="ce-secondary"
					onClick={() => {
						setState(emptyCache);
						setVersion("v1");
					}}
				>
					Reset
				</button>
			</div>
			<div className="ce-output" aria-live="polite" aria-atomic="true">
				<p>{state.last}</p>
				<dl className="ce-counts">
					<div>
						<dt>Compute branch runs</dt>
						<dd>{state.computations}</dd>
					</div>
					<div>
						<dt>Cache hits</dt>
						<dd>{state.hits}</dd>
					</div>
					<div>
						<dt>Simulated hour</dt>
						<dd>{state.clock}</dd>
					</div>
				</dl>
			</div>
			<p className="ce-note">
				A browser-only model of an explicit hit/miss branch. Counts are example
				actions, not performance measurements. Data resets when this page
				reloads.
			</p>
		</>
	);
}

function ChannelExplainer() {
	const [state, dispatch] = useReducer(channelTransition, emptyChannel);
	const waiting = state.status === "waiting";
	return (
		<>
			<h3>Send a request, then decide how it ends</h3>
			<p>
				A remote run asks a browser for a review decision. Try approval,
				rejection, or a timeout. An old response should not satisfy the next
				request.
			</p>
			<div className="ce-path" aria-label="Example participants">
				<span>Remote workflow</span>
				<span aria-hidden="true">↔</span>
				<span>Browser reviewer</span>
			</div>
			<div className="ce-actions">
				<button
					type="button"
					disabled={waiting}
					onClick={() => dispatch({ type: "start" })}
				>
					{state.request ? "Start next request" : "Start request"}
				</button>
				<button
					type="button"
					disabled={!waiting}
					onClick={() =>
						dispatch({ type: "reply", request: state.request, approved: true })
					}
				>
					Approve
				</button>
				<button
					type="button"
					className="ce-secondary"
					disabled={!waiting}
					onClick={() =>
						dispatch({ type: "reply", request: state.request, approved: false })
					}
				>
					Decline
				</button>
				<button
					type="button"
					className="ce-secondary"
					disabled={!waiting}
					onClick={() => dispatch({ type: "timeout" })}
				>
					End the wait
				</button>
				<button
					type="button"
					className="ce-secondary"
					disabled={!state.request}
					onClick={() =>
						dispatch({
							type: "reply",
							request: Math.max(0, state.request - 1),
							approved: true,
						})
					}
				>
					Send an old reply
				</button>
			</div>
			<div className="ce-output" aria-live="polite" aria-atomic="true">
				<p className="ce-summary">State: {state.status.replaceAll("-", " ")}</p>
				{state.log.length ? (
					<ol>
						{state.log.map((line, index) => (
							<li key={`${index}-${line}`}>{line}</li>
						))}
					</ol>
				) : (
					<p>Start a request to open a wait.</p>
				)}
			</div>
			<p className="ce-note">
				An interactive explanation using local sample state. It opens no channel
				and does not approve or execute a real workflow. Waiting is ended
				manually so you can inspect each outcome.
			</p>
		</>
	);
}

export function CommunityExplainer({
	mode,
}: { mode: "rss" | "cache" | "channels" }) {
	return (
		<section className="community-explainer" aria-label="Interactive example">
			<p className="ce-eyebrow">Try the idea · illustrative data</p>
			{mode === "rss" ? (
				<FeedExplainer />
			) : mode === "cache" ? (
				<CacheExplainer />
			) : (
				<ChannelExplainer />
			)}
		</section>
	);
}
