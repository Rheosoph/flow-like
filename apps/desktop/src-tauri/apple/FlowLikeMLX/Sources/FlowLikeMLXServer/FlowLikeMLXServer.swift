import FlowLikeMLX
import Foundation

private final class NDJSONWriter: @unchecked Sendable {
    private let lock = NSLock()

    func write(_ line: String) {
        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardOutput.write(Data((line + "\n").utf8))
    }

    func diagnostic(_ line: String) {
        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardError.write(Data((line + "\n").utf8))
    }
}

@main
private struct FlowLikeMLXServer {
    static func main() async {
        FlowLikeMLXRuntime.prepareForAppLifecycle()
        guard FlowLikeMLXRuntime.isAvailable else {
            FileHandle.standardError.write(
                Data("MLX requires an Apple-silicon Mac with Metal support\n".utf8)
            )
            return
        }

        let writer = NDJSONWriter()
        let decoder = JSONDecoder()

        while let line = readLine(strippingNewline: true) {
            guard let data = line.data(using: .utf8) else {
                writer.write(
                    FlowLikeMLXEventCodec.error(
                        id: "unknown",
                        message: "command is not valid UTF-8"
                    )
                )
                continue
            }

            let command: FlowLikeMLXCommand
            do {
                command = try decoder.decode(FlowLikeMLXCommand.self, from: data)
            } catch {
                writer.write(
                    FlowLikeMLXEventCodec.error(
                        id: recoverID(from: data),
                        message: "Invalid MLX command: \(error.localizedDescription)"
                    )
                )
                continue
            }

            switch command.command {
            case "generate":
                do {
                    _ = try command.validatedGenerate()
                    await FlowLikeMLXRuntime.shared.submit(command) { event in
                        writer.write(event)
                    }
                } catch {
                    writer.write(
                        FlowLikeMLXEventCodec.error(
                            id: command.id,
                            message: error.localizedDescription
                        )
                    )
                }

            case "cancel":
                await FlowLikeMLXRuntime.shared.cancel(requestID: command.id)

            case "unload":
                guard let directory = command.modelDirectory, !directory.isEmpty else {
                    writer.diagnostic(
                        "Ignoring unload command without model_directory"
                    )
                    continue
                }
                await FlowLikeMLXRuntime.shared.unload(modelDirectory: directory)

            case "clear_cache":
                await FlowLikeMLXRuntime.shared.clearCache()

            default:
                writer.write(
                    FlowLikeMLXEventCodec.error(
                        id: command.id,
                        message: "Unknown MLX command \"\(command.command)\""
                    )
                )
            }
        }

        // Normal Rust operation keeps stdin open for the sidecar lifetime. This
        // wait only matters for deliberate pipe/CLI use so accepted callbacks
        // are not lost when EOF arrives.
        await FlowLikeMLXRuntime.shared.waitUntilIdle()
    }

    private static func recoverID(from data: Data) -> String {
        guard let object = try? JSONSerialization.jsonObject(with: data),
            let dictionary = object as? [String: Any],
            let id = dictionary["id"] as? String,
            !id.isEmpty
        else {
            return "unknown"
        }
        return id
    }
}
