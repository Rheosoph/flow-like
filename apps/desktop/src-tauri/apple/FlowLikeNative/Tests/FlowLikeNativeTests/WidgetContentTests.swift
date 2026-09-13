import Foundation
import Testing
@testable import FlowLikeNative

@Test func widgetSelectionUsesCurrentTitleInsteadOfStoredConfiguration() throws {
    let selection = NativeWidgetAppSelection(snapshot: try widgetSnapshot(), appId: "app", scope: "alice")
    #expect(selection.state == .ready)
    #expect(selection.app?.title == "Current app title")
}

@Test func widgetVisibleDestinationHidesQueryValues() {
    #expect(NativeWidgetAppSelection.displayPath("/reports?token=private&name=Alice") == "/reports")
    #expect(NativeWidgetAppSelection.displayPath("?token=private") == "App home")
    #expect(NativeWidgetAppSelection.displayPath(nil) == "App home")
}

@Test func widgetSelectionHidesAppAfterAccountChange() throws {
    let selection = NativeWidgetAppSelection(snapshot: try widgetSnapshot(scope: "bob"), appId: "app", scope: "alice")
    #expect(selection.state == .unavailable)
    #expect(selection.app == nil)
}

@Test func widgetSelectionExpiresBeforeDisplayingPrivateAppDetails() throws {
    let selection = NativeWidgetAppSelection(snapshot: try widgetSnapshot(expires: .distantPast), appId: "app", scope: "alice")
    #expect(selection.state == .refresh)
    #expect(selection.app == nil)
    let signedOut = NativeWidgetAppSelection(snapshot: nil, appId: "app", scope: "alice")
    #expect(signedOut.state == .refresh)
    #expect(signedOut.app == nil)
}

@Test func widgetSelectionRejectsRemovedAndUnusableApps() throws {
    for selection in [
        NativeWidgetAppSelection(snapshot: try widgetSnapshot(), appId: "removed", scope: "alice"),
        NativeWidgetAppSelection(snapshot: try widgetSnapshot(eligible: false), appId: "app", scope: "alice"),
    ] {
        #expect(selection.state == .unavailable)
        #expect(selection.app == nil)
    }
}

@Test func widgetSelectionPromptsForAnAppWhenUnconfigured() throws {
    let selection = NativeWidgetAppSelection(snapshot: try widgetSnapshot(), appId: nil, scope: nil)
    #expect(selection.state == .chooseApp)
    #expect(selection.app == nil)
}

@Test func widgetLaunchURLPreservesPathAndLiteralQueryValues() throws {
    let path = "/orders?tab=recent+items"
    let value = "A&B + 50% / 東京 #1"
    let url = try NativeWidgetLaunchURL.app(appId: "app&1", scope: "workspace:alice",
                                            path: path, queryNames: ["search", "tag", "tag"],
                                            queryValues: [value, "first", "second"])
    #expect(url.absoluteString.contains("%2B"))
    #expect(!url.absoluteString.contains("+"))
    let components = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false))
    #expect(components.scheme == "flow-like")
    #expect(components.host == "native")
    #expect(components.path == "/app")
    let items = try #require(components.queryItems)
    #expect(items.first(where: { $0.name == "appId" })?.value == "app&1")
    #expect(items.first(where: { $0.name == "scope" })?.value == "workspace:alice")
    #expect(items.first(where: { $0.name == "path" })?.value == path)
    let encodedQuery = try #require(items.first(where: { $0.name == "queryParams" })?.value)
    let query = try JSONDecoder().decode([NativeQueryParameter].self, from: Data(encodedQuery.utf8))
    #expect(query == [NativeQueryParameter(name: "search", value: value),
                      NativeQueryParameter(name: "tag", value: "first"),
                      NativeQueryParameter(name: "tag", value: "second")])
}

@Test func widgetLaunchURLRejectsInvalidConfiguration() throws {
    #expect(throws: (any Error).self) {
        try NativeWidgetLaunchURL.app(appId: "app", scope: "alice", path: "https://example.com",
                                    queryNames: nil, queryValues: nil)
    }
    #expect(throws: (any Error).self) {
        try NativeWidgetLaunchURL.app(appId: "app", scope: "alice", path: nil,
                                    queryNames: ["search"], queryValues: [])
    }
}

@Test func runAndSectionWidgetURLsPreserveLiteralPlusInAccountScope() throws {
    let scope = "profile:alice+personal"
    for url in [NativeWidgetLaunchURL.run(appId: "app+1", runId: "run+2", scope: scope),
                NativeWidgetLaunchURL.section("flowpilot", scope: scope)] {
        #expect(!url.absoluteString.contains("+"))
        let components = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false))
        #expect(components.queryItems?.first(where: { $0.name == "scope" })?.value == scope)
    }
}

private func widgetSnapshot(scope: String = "alice", expires: Date = Date().addingTimeInterval(300),
                            eligible: Bool = true) throws -> NativeSnapshot {
    let data = try JSONSerialization.data(withJSONObject: [
        "version": 1,
        "scope": scope,
        "generatedAt": ISO8601DateFormatter().string(from: Date()),
        "expiresAt": ISO8601DateFormatter().string(from: expires),
        "sections": [],
        "events": [],
        "apps": [["id": "app", "title": "Current app title", "spotlightEligible": eligible]],
    ])
    return try JSONDecoder().decode(NativeSnapshot.self, from: data)
}
