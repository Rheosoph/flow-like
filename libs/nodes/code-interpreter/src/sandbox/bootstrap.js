import * as std from "std";
import * as os from "os";

const EXEC_DIR = "/flow";

const readJson = (name) => JSON.parse(std.loadFile(`${EXEC_DIR}/${name}`));
const config = readJson("config.json");
const inputs = readJson("inputs.json");
const userCode = std.loadFile(`${EXEC_DIR}/code.js`);

if (config.workdir) {
  os.chdir(config.workdir);
}

class CappedText {
  constructor(cap) {
    this.cap = cap;
    this.parts = [];
    this.kept = 0;
    this.dropped = 0;
  }

  write(text) {
    const room = this.cap == null ? Infinity : this.cap - this.kept;
    if (room <= 0) {
      this.dropped += text.length;
      return;
    }
    const piece = text.length > room ? text.slice(0, room) : text;
    this.dropped += text.length - piece.length;
    this.kept += piece.length;
    this.parts.push(piece);
  }

  value() {
    return this.parts.join("");
  }
}

const stdout = new CappedText(config.max_output_bytes);
const stderr = new CappedText(config.max_output_bytes);

const show = (value) => {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack || String(value);
  try {
    const json = JSON.stringify(value);
    return json === undefined ? String(value) : json;
  } catch (_) {
    return String(value);
  }
};

const logTo = (sink) => (...args) => sink.write(`${args.map(show).join(" ")}\n`);
const sandboxConsole = {
  log: logTo(stdout),
  info: logTo(stdout),
  debug: logTo(stdout),
  warn: logTo(stderr),
  error: logTo(stderr),
};

const outputs = {};
let error = null;

const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;

const finish = () => {
  const result = {
    outputs,
    stdout: stdout.value(),
    stderr: stderr.value(),
    error,
    success: error === null,
    stdout_dropped: stdout.dropped,
    stderr_dropped: stderr.dropped,
  };
  let text;
  try {
    text = JSON.stringify(result);
  } catch (e) {
    result.outputs = {};
    result.success = false;
    result.error = error ?? `outputs are not JSON-serialisable: ${e.message}`;
    text = JSON.stringify(result);
  }
  const file = std.open(`${EXEC_DIR}/outputs.json`, "w");
  file.puts(text);
  file.close();
};

try {
  const program = new AsyncFunction("inputs", "outputs", "console", userCode);
  await program(inputs, outputs, sandboxConsole);
} catch (e) {
  error = e && e.stack ? `${e.name}: ${e.message}\n${e.stack}` : String(e);
}

finish();
