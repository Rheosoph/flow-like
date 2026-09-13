import Foundation
import Testing
@testable import FlowLikeNative

@Test func customWidgetsRetainDataPastWorkspaceFreshnessButHideAfterOwnExpiry() throws {
    let now = Date()
    let snapshot = try customSnapshot([chartWidget(now: now)], snapshotExpiry: now.addingTimeInterval(-60))
    let current = NativeCustomWidgetSelection(snapshot: snapshot, id: "chart", scope: "alice", kind: "chart", now: now)
    #expect(current.state == .ready)
    #expect(current.widget?.isStale(at: now.addingTimeInterval(3600)) == true)
    let expired = NativeCustomWidgetSelection(snapshot: snapshot, id: "chart", scope: "alice", kind: "chart", now: now.addingTimeInterval(8 * 86_400))
    #expect(expired.state == .expired)
    #expect(expired.widget == nil)
}

@Test func customWidgetsNeverUseSavedNamesOrContentFromAnotherAccountOrKind() throws {
    let snapshot = try customSnapshot([chartWidget()])
    for selection in [
        NativeCustomWidgetSelection(snapshot: snapshot, id: "chart", scope: "bob", kind: "chart"),
        NativeCustomWidgetSelection(snapshot: snapshot, id: "removed", scope: "alice", kind: "chart"),
        NativeCustomWidgetSelection(snapshot: snapshot, id: "chart", scope: "alice", kind: "page"),
        NativeCustomWidgetSelection(snapshot: nil, id: "chart", scope: "alice", kind: "chart"),
    ] {
        #expect(selection.state == .unavailable)
        #expect(selection.widget == nil)
    }
    #expect(NativeCustomWidgetSelection(snapshot: snapshot, id: nil, scope: nil, kind: "chart").state == .chooseWidget)
}

@Test func customWidgetPublicationKeepsOtherWidgetsWhenAChartIsMalformed() throws {
    var invalid = chartWidget(id: "bad")
    invalid["chart"] = ["type": "line", "points": "not points"]
    let result = try publishCustomWidgets([invalid, chartWidget()])
    #expect(result.customWidgets?.count == 2)
    #expect(result.customWidgets?.first?.state == "error")
    #expect(result.customWidgets?.first?.chart == nil)
    #expect(result.customWidgets?.last?.state == "ready")
}

@Test func customWidgetPublicationDiscardsUnknownAppsAndBoundsItsCatalog() throws {
    var foreign = chartWidget(id: "foreign")
    foreign["appId"] = "other-app"
    let result = try publishCustomWidgets([foreign] + (0..<20).map { chartWidget(id: "widget-\($0)") })
    #expect(result.customWidgets?.count == 11)
    #expect(result.customWidgets?.allSatisfy { $0.appId == "app" } == true)
    let duplicate = try publishCustomWidgets([chartWidget(), chartWidget()])
    #expect(duplicate.customWidgets?.count == 1)
}

@Test func customWidgetPublicationRejectsExecutableActionsAndUnsafeNavigation() throws {
    var widget = chartWidget()
    widget["action"] = ["kind": "run_event", "appId": "app", "eventId": "private"]
    let result = try #require(try publishCustomWidgets([widget]).customWidgets?.first)
    #expect(result.state == "error")
    #expect(result.action == NativeAction(kind: "open_app", appId: "app"))
    widget["action"] = ["kind": "open_app", "appId": "app", "path": "https://example.com"]
    #expect(try publishCustomWidgets([widget]).customWidgets?.first?.state == "error")
}

@Test func customWidgetPublicationBoundsPageDepthAndRejectsCrossAppLinks() throws {
    var leaf: [String: Any] = ["id": "leaf", "kind": "text", "text": "Hello"]
    for level in 0..<9 { leaf = ["id": "level-\(level)", "kind": "column", "children": [leaf]] }
    var widget = chartWidget()
    widget["kind"] = "page"; widget.removeValue(forKey: "chart"); widget["page"] = leaf
    #expect(try publishCustomWidgets([widget]).customWidgets?.first?.state == "error")
    widget["page"] = ["id": "link", "kind": "link", "text": "View",
                      "action": ["kind": "open_app", "appId": "another", "path": "/"]]
    #expect(try publishCustomWidgets([widget]).customWidgets?.first?.state == "error")
}

@Test func customWidgetPublicationValidatesChartAndTableDisplayBounds() throws {
    var widget = chartWidget()
    var chart = widget["chart"] as! [String: Any]
    chart["points"] = (0..<121).map { ["id": "\($0)", "label": "Day", "series": "Sales", "value": $0, "formattedValue": "\($0)"] }
    widget["chart"] = chart
    #expect(try publishCustomWidgets([widget]).customWidgets?.first?.state == "error")
    widget["kind"] = "page"; widget.removeValue(forKey: "chart")
    widget["page"] = ["id": "table", "kind": "table", "table": ["columns": ["Name", "Value"], "rows": [["Only one cell"]]]]
    #expect(try publishCustomWidgets([widget]).customWidgets?.first?.state == "error")
}

@Test func customWidgetPublicationKeepsValidNativePageAndDropsUnknownAuthorityFields() throws {
    var widget = chartWidget()
    widget["kind"] = "page"; widget.removeValue(forKey: "chart")
    widget["capability"] = "not-a-widget-authority"
    widget["page"] = ["id": "root", "kind": "column", "children": [
        ["id": "title", "kind": "text", "text": "Ready", "role": "title"],
        ["id": "badge", "kind": "badge", "text": "On track", "tone": "success"],
        ["id": "progress", "kind": "progress", "progress": 0.8],
        ["id": "image", "kind": "icon", "image": ["text": "🎉"], "src": "https://private.example/image"],
    ]]
    let snapshot = try publishCustomWidgets([widget])
    let result = try #require(snapshot.customWidgets?.first)
    #expect(result.state == "ready")
    #expect(result.page?.children?.last?.image?.text == "🎉")
    let json = String(decoding: try JSONEncoder().encode(snapshot), as: UTF8.self)
    #expect(!json.contains("not-a-widget-authority"))
    #expect(!json.contains("private.example"))
}

@Test func customWidgetLinkPreservesLiteralQueryValues() throws {
    var widget = chartWidget()
    widget["action"] = ["kind": "open_app", "appId": "app", "path": "/orders", "queryParams": [["name": "filter", "value": "A&B + 東京"]]]
    let result = try #require(try publishCustomWidgets([widget]).customWidgets?.first)
    let url = result.launchURL(scope: "alice+profile")
    #expect(!url.absoluteString.contains("+"))
    let values = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
    #expect(values.first(where: { $0.name == "scope" })?.value == "alice+profile")
    #expect(values.first(where: { $0.name == "queryParams" })?.value?.contains("A&B + 東京") == true)
}

private func chartWidget(id: String = "chart", now: Date = Date()) -> [String: Any] {
    let iso = ISO8601DateFormatter()
    return [
        "id": id, "title": "Revenue", "kind": "chart", "appId": "app", "state": "ready", "accent": "teal",
        "updatedAt": iso.string(from: now), "staleAt": iso.string(from: now.addingTimeInterval(1800)),
        "expiresAt": iso.string(from: now.addingTimeInterval(7 * 86_400)),
        "action": ["kind": "open_app", "appId": "app", "path": "/reports"],
        "chart": ["type": "line", "format": ["style": "currency", "currency": "EUR", "decimals": 2],
                  "points": [["id": "one", "label": "Monday", "series": "Sales", "value": 120.5, "formattedValue": "€120.50"]]],
    ]
}

private func customSnapshot(_ widgets: [[String: Any]], snapshotExpiry: Date = Date().addingTimeInterval(3600)) throws -> NativeSnapshot {
    let iso = ISO8601DateFormatter()
    return try JSONDecoder().decode(NativeSnapshot.self, from: JSONSerialization.data(withJSONObject: [
        "version": 1, "scope": "alice", "generatedAt": iso.string(from: Date()), "expiresAt": iso.string(from: snapshotExpiry),
        "apps": [["id": "app", "title": "App"]], "sections": [], "events": [], "customWidgets": widgets,
    ]))
}

private func publishCustomWidgets(_ widgets: [[String: Any]]) throws -> NativeSnapshot {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    return try NativeStore(directory: directory).publish(JSONEncoder().encode(customSnapshot(widgets)))
}
