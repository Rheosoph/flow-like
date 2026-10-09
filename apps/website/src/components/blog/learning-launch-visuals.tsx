import { type ReactNode, useId, useState } from "react";
import "./learning-launch-visuals.css";

function VisualFrame({
	eyebrow,
	title,
	intro,
	children,
	caption,
}: {
	eyebrow: string;
	title: string;
	intro: string;
	children: ReactNode;
	caption: string;
}) {
	const id = useId();
	return (
		<figure className="learning-visual not-prose" aria-labelledby={id}>
			<header className="lv-header">
				<p className="lv-eyebrow">{eyebrow}</p>
				<h3 id={id}>{title}</h3>
				<p className="lv-intro">{intro}</p>
			</header>
			{children}
			<figcaption className="lv-caption">{caption}</figcaption>
		</figure>
	);
}

function Choices({
	label,
	options,
	selected,
	onSelect,
	controls,
}: {
	label: string;
	options: readonly string[];
	selected: number;
	onSelect: (index: number) => void;
	controls: string;
}) {
	return (
		<fieldset className="lv-choices" aria-label={label}>
			{options.map((option, index) => (
				<button
					key={option}
					type="button"
					aria-pressed={selected === index}
					aria-controls={controls}
					onClick={() => onSelect(index)}
				>
					{option}
				</button>
			))}
		</fieldset>
	);
}

function FlowPath({
	steps,
}: { steps: readonly { label: string; detail: string }[] }) {
	return (
		<ol className="lv-flow">
			{steps.map((step, index) => (
				<li key={step.label}>
					<span className="lv-step-number" aria-hidden="true">
						{String(index + 1).padStart(2, "0")}
					</span>
					<strong>{step.label}</strong>
					<span>{step.detail}</span>
				</li>
			))}
		</ol>
	);
}

const trainingStages = [
	{
		label: "Prepare",
		title: "Give every experiment the same starting point",
		text: "Pin the source tables, define the prediction target, and preserve row groups across the split. Fit preprocessing on training rows and carry it with the model.",
		steps: [
			{ label: "Pin data", detail: "Table branch and version" },
			{ label: "Set the task", detail: "Target, metric and budget" },
			{ label: "Split rows", detail: "Group or time boundaries" },
			{ label: "Prepare", detail: "Fit on training rows" },
		],
		result:
			"An experiment has a fixed task, reproducible inputs and a bounded search.",
	},
	{
		label: "Search",
		title: "Let evidence guide the next candidate",
		text: "The controller trains and scores candidates. An optional language model consultant can suggest configurations or features using the training profile and validation history.",
		steps: [
			{ label: "Choose", detail: "Candidate and features" },
			{ label: "Train", detail: "Within the resource budget" },
			{ label: "Validate", detail: "Controller measures results" },
			{ label: "Decide", detail: "Continue, refine or finish" },
		],
		result:
			"Validation informs the search. The final test partition stays outside the loop.",
	},
	{
		label: "Audit",
		title: "Freeze the winner before opening the final test",
		text: "Select the model from validation evidence, then evaluate that fixed model on independent test rows. Report its measured result against the quality goal.",
		steps: [
			{ label: "Select", detail: "Validation winner" },
			{ label: "Freeze", detail: "Model choice is fixed" },
			{ label: "Evaluate", detail: "Independent test partition" },
			{ label: "Report", detail: "Metrics and goal status" },
		],
		result:
			"The final test reports how the selected model performs on independent examples.",
	},
	{
		label: "Use",
		title: "Bring the trained model back into the workflow",
		text: "Pass the saved experiment ID to Predict Auto Model. It loads the selected artifact and applies its saved feature mapping and preprocessing to new rows.",
		steps: [
			{ label: "Save", detail: "Model and preprocessing" },
			{ label: "Load", detail: "Saved experiment ID" },
			{ label: "Predict", detail: "Apply the fitted pipeline" },
			{ label: "Act", detail: "Your downstream workflow" },
		],
		result:
			"The workflow keeps the connection between the experiment and the model it uses.",
	},
] as const;

export function TrainingAgentExplorer() {
	const [selected, setSelected] = useState(0);
	const id = useId();
	const stage = trainingStages[selected];
	return (
		<VisualFrame
			eyebrow="Inside the training agent"
			title="One experiment. A clear path to a model."
			intro="Select a stage to follow the data, the search and the final decision."
			caption="Training fits the model. Validation guides selection. The final test evaluates the frozen winner."
		>
			<Choices
				label="Experiment stage"
				options={trainingStages.map((item) => item.label)}
				selected={selected}
				onSelect={setSelected}
				controls={id}
			/>
			<div className="lv-partitions">
				<div className="lv-partition" data-active={selected < 2}>
					<span className="lv-partition-mark" aria-hidden="true">
						▤
					</span>
					<strong>Training</strong>
					<span>Learn from examples</span>
				</div>
				<div className="lv-partition" data-active={selected === 1}>
					<span className="lv-partition-mark" aria-hidden="true">
						◎
					</span>
					<strong>Validation</strong>
					<span>Compare candidates</span>
				</div>
				<div
					className="lv-partition lv-partition-test"
					data-active={selected === 2}
				>
					<svg
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.6"
						aria-hidden="true"
					>
						<rect x="5" y="10" width="14" height="11" rx="2" />
						<path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3" />
					</svg>
					<strong>Final test</strong>
					<span>Open after selection</span>
				</div>
			</div>
			<div className="lv-detail" id={id} aria-live="polite" aria-atomic="true">
				<p className="lv-section-label">
					{`${String(selected + 1).padStart(2, "0")} / ${stage.label}`}
				</p>
				<h4>{stage.title}</h4>
				<p>{stage.text}</p>
				<FlowPath steps={stage.steps} />
				<p className="lv-result">{stage.result}</p>
			</div>
		</VisualFrame>
	);
}

const learningTasks = [
	{
		label: "Tabular",
		title: "Turn named features into a prediction",
		input: "Measurements and categories",
		shape: "One feature vector per sample",
		models: [
			"MLP",
			"Histogram Gradient Booster",
			"Decision Tree",
			"Logistic Regression",
		],
		output: "A class or numeric estimate",
		example:
			"Use pressure, temperature and part attributes to predict a quality outcome.",
		steps: [
			{ label: "Map features", detail: "Numeric and categorical columns" },
			{ label: "Train", detail: "Fit a model to labeled rows" },
			{ label: "Evaluate", detail: "Measure independent predictions" },
			{ label: "Predict", detail: "Reuse training preprocessing" },
		],
	},
	{
		label: "Time series",
		title: "Keep the order of your observations",
		input: "Windows of sensor readings",
		shape: "Time steps × sensor channels",
		models: ["LSTM", "GRU", "1D CNN", "TCN"],
		output: "A forecast or window classification",
		example:
			"Use a window of vibration readings to classify the operating state of a machine.",
		steps: [
			{ label: "Window", detail: "Preserve temporal order" },
			{ label: "Train", detail: "Learn from sequences" },
			{ label: "Evaluate", detail: "Keep related windows together" },
			{ label: "Predict", detail: "Process the next window" },
		],
	},
	{
		label: "Vision",
		title: "Choose the visual answer your workflow needs",
		input: "Images and task annotations",
		shape: "Image tensors with labels, boxes or masks",
		models: [
			"ResNet-18",
			"MobileNetV2",
			"EfficientNet",
			"YOLOX",
			"U-Net",
			"Mask R-CNN",
		],
		output: "Image classes, boxes or masks",
		example:
			"Classify a part, locate a surface defect, or mark the pixels that belong to it.",
		steps: [
			{ label: "Prepare", detail: "Decode and preprocess images" },
			{ label: "Train", detail: "Choose the vision task" },
			{ label: "Evaluate", detail: "Use task-specific metrics" },
			{ label: "Inspect", detail: "Route the resulting prediction" },
		],
	},
	{
		label: "Anomalies",
		title: "Learn the patterns of normal operation",
		input: "Sensor features or visual features",
		shape: "Vectors, sequences or feature maps",
		models: [
			"Isolation Forest",
			"Autoencoders",
			"PatchCore",
			"PaDiM",
			"EfficientAD",
		],
		output: "Anomaly scores and visual maps",
		example:
			"Find unusual equipment behavior or localize image regions that differ from normal examples.",
		steps: [
			{ label: "Prepare", detail: "Build compatible input features" },
			{ label: "Fit", detail: "Model normal patterns" },
			{ label: "Calibrate", detail: "Select a validation threshold" },
			{ label: "Detect", detail: "Route unusual observations" },
		],
	},
] as const;

function ModelGlyph({ variant }: { variant: number }) {
	const layers = [
		[42, 64, 86, 108],
		[32, 54, 76, 98, 120],
		[42, 64, 86, 108],
		[54, 98],
	];
	return (
		<svg
			className="lv-model-glyph"
			viewBox="0 0 280 150"
			fill="none"
			aria-hidden="true"
		>
			{layers
				.slice(0, -1)
				.flatMap((layer, layerIndex) =>
					layer.flatMap((y) =>
						layers[layerIndex + 1].map((nextY) => (
							<path
								key={`${layerIndex}-${y}-${nextY}`}
								d={`M${40 + layerIndex * 66} ${y} L${106 + layerIndex * 66} ${nextY}`}
								stroke="currentColor"
								opacity={(y + nextY + variant) % 3 === 0 ? 0.5 : 0.13}
							/>
						)),
					),
				)}
			{layers.flatMap((layer, layerIndex) =>
				layer.map((y) => (
					<circle
						key={`${layerIndex}-${y}`}
						cx={40 + layerIndex * 66}
						cy={y}
						r={layerIndex === 3 ? 7 : 5}
						fill={layerIndex === 3 ? "#f5c47a" : "#0d2c3b"}
						stroke={layerIndex === 3 ? "#f5c47a" : "currentColor"}
					/>
				)),
			)}
		</svg>
	);
}

export function LearningNodesExplorer() {
	const [selected, setSelected] = useState(0);
	const id = useId();
	const task = learningTasks[selected];
	return (
		<VisualFrame
			eyebrow="Explore the model families"
			title="Start with the shape of your problem."
			intro="Choose a task to see its inputs, model families and path into a workflow."
			caption="Each family has its own training recipe. The workflow connects preparation, evaluation and inference."
		>
			<Choices
				label="Machine learning task"
				options={learningTasks.map((item) => item.label)}
				selected={selected}
				onSelect={setSelected}
				controls={id}
			/>
			<div className="lv-detail" id={id} aria-live="polite" aria-atomic="true">
				<div className="lv-model-panel">
					<div>
						<p className="lv-section-label">{`${task.label} / Model families`}</p>
						<h4>{task.title}</h4>
						<p>{task.example}</p>
					</div>
					<ModelGlyph variant={selected} />
				</div>
				<div className="lv-model-io">
					<div>
						<span className="lv-section-label">Input</span>
						<strong>{task.input}</strong>
						<span>{task.shape}</span>
					</div>
					<span className="lv-io-arrow" aria-hidden="true">
						→
					</span>
					<div>
						<span className="lv-section-label">Output</span>
						<strong>{task.output}</strong>
						<span>The prediction your next node receives</span>
					</div>
				</div>
				<ul className="lv-chips" aria-label="Available model families">
					{task.models.map((model) => (
						<li key={model}>{model}</li>
					))}
				</ul>
				<FlowPath steps={task.steps} />
			</div>
		</VisualFrame>
	);
}

const protocolPaths = [
	{
		label: "Machine data",
		title: "Give a register value its business context",
		protocols: ["Modbus TCP / RTU", "IO-Link", "HART"],
		source: "Machine or instrument",
		connection: "Read / poll",
		text: "Read equipment values, decode the device's representation, then add the units and identifiers that make the reading useful downstream.",
		steps: [
			{ label: "Connect", detail: "Device endpoint or adapter" },
			{ label: "Read", detail: "Registers or process values" },
			{ label: "Enrich", detail: "Units, asset and timestamp" },
			{ label: "Use", detail: "Store, alert or predict" },
		],
		boundary:
			"For Modbus, the register map determines addresses, data types, byte order and word order.",
	},
	{
		label: "PLC events",
		title: "Make controller changes part of the process",
		protocols: ["OPC UA", "ADS / TwinCAT", "EtherNet/IP / CIP"],
		source: "PLC or server",
		connection: "Subscribe / poll",
		text: "Use OPC UA monitored items, ADS notifications or Logix tag polling to bring controller state into a workflow function.",
		steps: [
			{ label: "Connect", detail: "Session and device identity" },
			{ label: "Observe", detail: "Subscriptions or tag polling" },
			{ label: "Handle", detail: "Referenced workflow function" },
			{ label: "Route", detail: "Equipment or batch event" },
		],
		boundary:
			"The workflow reuses the connection during its run. OPC UA uses the configured certificate trust store.",
	},
	{
		label: "Messaging",
		title: "Connect the shop floor to the next system",
		protocols: [
			"MQTT / Sparkplug B",
			"NATS / JetStream",
			"Kafka / Redpanda",
			"RabbitMQ",
			"Redis Streams",
			"Zenoh",
			"Iroh",
		],
		source: "Broker or peer",
		connection: "Consume / subscribe",
		text: "Route messages into a workflow handler. JetStream, RabbitMQ, Redis Streams and Kafka acknowledge or commit after the handler succeeds.",
		steps: [
			{ label: "Receive", detail: "Broker message or peer event" },
			{ label: "Handle", detail: "Process the event" },
			{ label: "Complete", detail: "Finish the downstream work" },
			{ label: "Confirm", detail: "Queue acknowledgement or commit" },
		],
		boundary:
			"The acknowledgement path shown applies to JetStream, RabbitMQ, Redis Streams and Kafka. Use a stable business key to handle redelivery.",
	},
	{
		label: "Fieldbus cycles",
		title: "Bring cyclic process data into the workflow",
		protocols: ["EtherCAT", "PROFIBUS DP-V0", "PROFINET via cifX"],
		source: "Fieldbus devices",
		connection: "Cycle / process image",
		text: "EtherCAT and PROFIBUS run bus cycles on dedicated workers. PROFINET process data comes through an installed Hilscher cifX controller and its native driver.",
		steps: [
			{ label: "Connect", detail: "Interface or commissioned card" },
			{ label: "Exchange", detail: "Worker or controller cycle" },
			{ label: "Snapshot", detail: "Read process data" },
			{ label: "Handle", detail: "Analyze in the workflow" },
		],
		boundary:
			"The workflow handles snapshots while the dedicated worker or hardware controller manages the bus exchange.",
	},
] as const;

export function IndustrialProtocolExplorer() {
	const [selected, setSelected] = useState(0);
	const id = useId();
	const path = protocolPaths[selected];
	return (
		<VisualFrame
			eyebrow="From equipment to workflow"
			title="Keep the protocol. Connect the process."
			intro="Explore four ways to bring industrial data into Flow-Like."
			caption="Choose the protocol role and executor access that match the equipment you already operate."
		>
			<Choices
				label="Industrial connection pattern"
				options={protocolPaths.map((item) => item.label)}
				selected={selected}
				onSelect={setSelected}
				controls={id}
			/>
			<div className="lv-detail" id={id} aria-live="polite" aria-atomic="true">
				<div className="lv-connection-map">
					<div className="lv-equipment">
						<svg
							viewBox="0 0 72 62"
							fill="none"
							stroke="currentColor"
							strokeWidth="1.5"
							aria-hidden="true"
						>
							<rect x="7" y="12" width="58" height="42" rx="5" />
							<rect x="14" y="20" width="25" height="19" rx="2" />
							<path d="M20 46h13M45 25h12M45 32h12M45 39h12M18 6v6M28 6v6M44 54v6M54 54v6" />
							<circle cx="20" cy="29" r="3" fill="currentColor" />
						</svg>
						<strong>{path.source}</strong>
					</div>
					<div className="lv-signal">
						<span>{path.connection}</span>
						<span className="lv-signal-line" aria-hidden="true" />
					</div>
					<div className="lv-workflow-mark">
						<svg
							viewBox="0 0 72 62"
							fill="none"
							stroke="currentColor"
							strokeWidth="1.5"
							aria-hidden="true"
						>
							<path d="M24 31h12V14h12M36 31v17h12" />
							<rect x="6" y="22" width="18" height="18" rx="4" />
							<rect x="48" y="5" width="18" height="18" rx="4" />
							<rect x="48" y="39" width="18" height="18" rx="4" />
						</svg>
						<strong>Flow-Like workflow</strong>
					</div>
				</div>
				<h4>{path.title}</h4>
				<p>{path.text}</p>
				<ul
					className="lv-chips"
					aria-label="Protocols for this connection pattern"
				>
					{path.protocols.map((protocol) => (
						<li key={protocol}>{protocol}</li>
					))}
				</ul>
				<FlowPath steps={path.steps} />
				<p className="lv-result">{path.boundary}</p>
			</div>
		</VisualFrame>
	);
}
