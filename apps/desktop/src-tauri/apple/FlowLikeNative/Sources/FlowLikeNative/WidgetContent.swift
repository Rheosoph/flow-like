import Foundation

public struct NativeWidgetAppSelection: Sendable {
    public static func displayPath(_ path: String?) -> String {
        let route = String((path ?? "").split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)[0])
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return route.isEmpty ? "App home" : route
    }

    public enum State: Sendable, Equatable {
        case ready
        case chooseApp
        case refresh
        case unavailable
    }

    public let state: State
    public let app: NativeApp?

    public init(snapshot: NativeSnapshot?, appId: String?, scope: String?, now: Date = Date()) {
        guard let snapshot, snapshot.version == 1,
              let expiry = NativeSnapshot.date(snapshot.expiresAt), expiry > now else {
            state = .refresh
            app = nil
            return
        }
        guard let appId, let scope else {
            state = .chooseApp
            app = nil
            return
        }
        guard snapshot.scope == scope, !scope.isEmpty,
              let current = snapshot.apps.first(where: { $0.id == appId && $0.spotlightEligible == true }) else {
            state = .unavailable
            app = nil
            return
        }
        state = .ready
        app = current
    }
}

public enum NativeWidgetLaunchURL {
    public static let home = URL(string: "flow-like://native/home")!

    public static func app(appId: String, scope: String, path: String?,
                           queryNames: [String]?, queryValues: [String]?) throws -> URL {
        let query = try NativeAppRoute.queryParameters(names: queryNames, values: queryValues)
        try NativeAppRoute.validate(path: path, queryParams: query)
        var components = URLComponents(string: "flow-like://native/app")!
        components.queryItems = [URLQueryItem(name: "appId", value: appId),
                                 URLQueryItem(name: "scope", value: scope)]
        if let path, !path.isEmpty {
            components.queryItems?.append(URLQueryItem(name: "path", value: path))
        }
        if !query.isEmpty {
            let data = try JSONEncoder().encode(query)
            components.queryItems?.append(URLQueryItem(name: "queryParams", value: String(decoding: data, as: UTF8.self)))
        }
        // The frontend parses query strings as form data, where a literal + means a space.
        components.percentEncodedQuery = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        guard let url = components.url else { throw NativeIntegrationError.invalidAction }
        return url
    }

    public static func section(_ kind: String, scope: String?) -> URL {
        guard let scope, !scope.isEmpty else { return home }
        let destination: String
        switch kind {
        case "flowpilot": destination = "flowpilot"
        case "inbox", "attention": destination = "inbox"
        default: return home
        }
        var components = URLComponents(string: "flow-like://native/\(destination)")!
        components.queryItems = [URLQueryItem(name: "scope", value: scope)]
        components.percentEncodedQuery = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        return components.url ?? home
    }

    public static func run(appId: String, runId: String, scope: String) -> URL {
        var components = URLComponents(string: "flow-like://native/run")!
        components.queryItems = [URLQueryItem(name: "appId", value: appId),
                                 URLQueryItem(name: "runId", value: runId),
                                 URLQueryItem(name: "scope", value: scope)]
        components.percentEncodedQuery = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        return components.url ?? home
    }
}
