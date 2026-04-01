// Flow-Like WASM Node Template (MoonBit)
//
// Build:
//   moon build --target wasm --release
//
// The compiled .wasm file will be at: build/node.wasm

// ---------------------------------------------------------------------------
// Node definition
// ---------------------------------------------------------------------------

fn get_definition() -> @sdk.NodeDefinition {
  let def = @sdk.NodeDefinition::new(
    "my_custom_node_moonbit",
    "My Custom Node (MoonBit)",
    "A template WASM node built with MoonBit",
    "Custom/WASM",
  )
  def.add_permission("streaming")

  def.add_pin(@sdk.input_pin("exec", "Execute", "Trigger execution", @sdk.data_type_exec()))
  def.add_pin(
    @sdk.input_pin("input_text", "Input Text", "Text to process", @sdk.data_type_string())
      .with_default("\"\""),
  )
  def.add_pin(
    @sdk.input_pin("multiplier", "Multiplier", "Number of times to repeat", @sdk.data_type_i64())
      .with_default("1"),
  )
  def.add_pin(@sdk.output_pin("exec_out", "Done", "Execution complete", @sdk.data_type_exec()))
  def.add_pin(@sdk.output_pin("output_text", "Output Text", "Processed text", @sdk.data_type_string()))
  def.add_pin(@sdk.output_pin("char_count", "Character Count", "Number of characters in output", @sdk.data_type_i64()))

  def
}

// ---------------------------------------------------------------------------
// Run logic
// ---------------------------------------------------------------------------

fn handle_run(ctx : @sdk.Context) -> @sdk.ExecutionResult {
  let input_text = ctx.get_string("input_text")
  let multiplier = ctx.get_i64("multiplier", default=1)

  let mut output_text = ""
  for _i = 0; _i < multiplier; _i = _i + 1 {
    output_text = output_text + input_text
  }

  let char_count = output_text.length()

  ctx.stream_text("Generated " + char_count.to_string() + " characters")

  ctx.set_output("output_text", @sdk.json_string(output_text))
  ctx.set_output("char_count", char_count.to_string())

  ctx.success()
}

// ---------------------------------------------------------------------------
// Exports (called by the host runtime)
// ---------------------------------------------------------------------------

pub fn get_node() -> Int64 {
  @sdk.serialize_definition(get_definition())
}

pub fn get_nodes() -> Int64 {
  let json = "[" + get_definition().to_json() + "]"
  @sdk.serialize_to_mem(json)
}

pub fn run(ptr : Int, len : Int) -> Int64 {
  let input = @sdk.parse_input(ptr, len)
  let ctx = @sdk.Context::new(input)
  let result = handle_run(ctx)
  @sdk.serialize_result(result)
}

pub fn alloc(size : Int) -> Int {
  @sdk.alloc(size)
}

pub fn dealloc(ptr : Int, size : Int) -> Unit {
  @sdk.dealloc(ptr, size)
}

pub fn get_abi_version() -> Int {
  @sdk.abi_version
}

fn main {
  ()
}
