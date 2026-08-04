import Foundation

public typealias FlowLikeMLXCEventCallback =
    @convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void

private final class FlowLikeMLXCCallbackTarget: @unchecked Sendable {
    private let callback: FlowLikeMLXCEventCallback
    private let context: UnsafeMutableRawPointer

    init(
        callback: @escaping FlowLikeMLXCEventCallback,
        context: UnsafeMutableRawPointer
    ) {
        self.callback = callback
        self.context = context
    }

    func send(_ event: String) {
        event.withCString { pointer in
            callback(pointer, context)
        }
    }
}

/// Returns 1 only on a physical Apple-silicon device with a usable Metal GPU.
@_cdecl("flow_like_mlx_is_available")
public func flow_like_mlx_is_available() -> Int32 {
    guard FlowLikeMLXRuntime.isAvailable else { return 0 }
    FlowLikeMLXRuntime.prepareForAppLifecycle()
    return 1
}

/// Starts an in-process iOS generation.
///
/// ABI contract with Rust:
/// - nonzero return: the callback is never retained or invoked;
/// - zero return: exactly one terminal `complete` or `error` event is invoked;
/// - callback calls are serialized and no call occurs after the terminal call.
@_cdecl("flow_like_mlx_generate")
public func flow_like_mlx_generate(
    _ requestJSON: UnsafePointer<CChar>?,
    _ callback: @escaping FlowLikeMLXCEventCallback,
    _ context: UnsafeMutableRawPointer?
) -> Int32 {
    guard let requestJSON, let context else { return 1 }

    // Rust keeps the CString alive for this synchronous bridge call. Decode
    // directly from that storage so large requests (notably base64 VLM images)
    // do not also exist as a Swift String and a copied Data buffer.
    let data = Data(
        bytesNoCopy: UnsafeMutableRawPointer(mutating: requestJSON),
        count: strlen(requestJSON),
        deallocator: .none
    )
    guard let command = try? JSONDecoder().decode(FlowLikeMLXCommand.self, from: data),
        (try? command.validatedGenerate()) != nil
    else {
        // Rust owns and will reclaim `context` when a nonzero result is
        // returned. Invoking the callback in this branch would double-free it.
        return 2
    }

    let target = FlowLikeMLXCCallbackTarget(
        callback: callback,
        context: context
    )
    Task {
        await FlowLikeMLXRuntime.shared.submit(command) { event in
            target.send(event)
        }
    }
    return 0
}

@_cdecl("flow_like_mlx_cancel")
public func flow_like_mlx_cancel(_ requestID: UnsafePointer<CChar>?) {
    guard let requestID else { return }
    let id = String(cString: requestID)
    Task {
        await FlowLikeMLXRuntime.shared.cancel(requestID: id)
    }
}

@_cdecl("flow_like_mlx_unload")
public func flow_like_mlx_unload(_ modelDirectory: UnsafePointer<CChar>?) {
    guard let modelDirectory else { return }
    let directory = String(cString: modelDirectory)
    Task {
        await FlowLikeMLXRuntime.shared.unload(modelDirectory: directory)
    }
}

@_cdecl("flow_like_mlx_clear_cache")
public func flow_like_mlx_clear_cache() {
    Task {
        await FlowLikeMLXRuntime.shared.clearCache()
    }
}
