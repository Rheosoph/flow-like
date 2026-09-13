import Foundation

public struct NativeWidgetChart: Codable, Sendable {
    public struct Point: Codable, Sendable, Identifiable {
        public var id: String
        public var label: String
        public var series: String
        public var value: Double
        public var formattedValue: String
    }
    public struct Format: Codable, Sendable {
        public var style: String
        public var currency: String
        public var decimals: Int
    }
    public var type: String
    public var points: [Point]
    public var format: Format
    public var target: Double?
    public var value: Double?
    public var formattedValue: String?

    func validate() throws {
        guard ["stat", "progress", "gauge", "bar", "horizontal", "stacked", "line", "area", "donut", "pie"].contains(type),
              points.count <= 120, Set(points.map(\.series)).count <= 6,
              Set(points.map(\.id)).count == points.count,
              ["number", "currency", "percent"].contains(format.style),
              format.currency.utf8.count <= 8, (0...6).contains(format.decimals),
              value?.isFinite != false, target?.isFinite != false,
              (formattedValue?.utf8.count ?? 0) <= 160 else { throw NativeIntegrationError.invalidAction }
        for point in points {
            guard !point.id.isEmpty, point.id.utf8.count <= 256, point.label.utf8.count <= 512,
                  point.series.utf8.count <= 512, point.formattedValue.utf8.count <= 160,
                  point.value.isFinite,
                  !["pie", "donut"].contains(type) || point.value >= 0 else { throw NativeIntegrationError.invalidAction }
        }
    }
}

public struct NativeWidgetPageNode: Codable, Sendable, Identifiable {
    public struct Table: Codable, Sendable {
        public var columns: [String]
        public var rows: [[String]]
    }
    public var id: String
    public var kind: String
    public var text: String?
    public var role: String?
    public var tone: String?
    public var alignment: String?
    public var spacing: Double?
    public var columns: Int?
    public var children: [NativeWidgetPageNode]?
    public var image: NativeNotificationIcon?
    public var progress: Double?
    public var value: String?
    public var table: Table?
    public var chart: NativeWidgetChart?
    public var action: NativeAction?

    fileprivate mutating func validate(appId: String, depth: Int, nodes: inout Set<String>, images: inout Int) throws {
        guard depth <= 8, nodes.count < 60, !id.isEmpty, id.utf8.count <= 256, nodes.insert(id).inserted,
              ["column", "row", "stack", "grid", "card", "text", "image", "icon", "badge", "progress", "divider", "spacer", "table", "chart", "link"].contains(kind),
              (text?.utf8.count ?? 0) <= 4096, (value?.utf8.count ?? 0) <= 256,
              role == nil || ["title", "headline", "body", "caption"].contains(role!),
              tone == nil || ["default", "muted", "accent", "success", "warning", "danger"].contains(tone!),
              alignment == nil || ["leading", "center", "trailing"].contains(alignment!),
              spacing == nil || (spacing!.isFinite && (0...32).contains(spacing!)),
              columns == nil || (1...6).contains(columns!),
              progress == nil || (progress!.isFinite && (0...1).contains(progress!)) else { throw NativeIntegrationError.invalidAction }
        if let action { try NativeCustomWidget.validateAction(action, appId: appId) }
        if image != nil {
            images += 1
            guard images <= 4 else { throw NativeIntegrationError.oversized }
            image = image?.normalized()
        }
        if kind == "chart" {
            guard let chart else { throw NativeIntegrationError.invalidAction }
            try chart.validate()
        }
        if kind == "table" {
            guard let table, !table.columns.isEmpty, table.columns.count <= 6, table.rows.count <= 8,
                  table.columns.allSatisfy({ $0.utf8.count <= 160 }),
                  table.rows.allSatisfy({ $0.count == table.columns.count && $0.allSatisfy({ $0.utf8.count <= 256 }) }) else {
                throw NativeIntegrationError.invalidAction
            }
        }
        for index in children?.indices ?? 0..<0 {
            try children![index].validate(appId: appId, depth: depth + 1, nodes: &nodes, images: &images)
        }
    }
}

public struct NativeCustomWidget: Codable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var kind: String
    public var appId: String
    public var updatedAt: String
    public var staleAt: String
    public var expiresAt: String
    public var state: String
    public var message: String?
    public var warnings: [String]?
    public var accent: String?
    public var action: NativeAction
    public var chart: NativeWidgetChart?
    public var page: NativeWidgetPageNode?

    private enum CodingKeys: String, CodingKey {
        case id, title, kind, appId, updatedAt, staleAt, expiresAt, state, message, warnings, accent, action, chart, page
    }

    // A malformed custom widget must not discard the account's other widgets.
    public init(from decoder: Decoder) throws {
        let values = try? decoder.container(keyedBy: CodingKeys.self)
        id = (try? values?.decode(String.self, forKey: .id)) ?? ""
        title = (try? values?.decode(String.self, forKey: .title)) ?? "Widget"
        kind = (try? values?.decode(String.self, forKey: .kind)) ?? ""
        appId = (try? values?.decode(String.self, forKey: .appId)) ?? ""
        updatedAt = (try? values?.decode(String.self, forKey: .updatedAt)) ?? ""
        staleAt = (try? values?.decode(String.self, forKey: .staleAt)) ?? ""
        expiresAt = (try? values?.decode(String.self, forKey: .expiresAt)) ?? ""
        state = (try? values?.decode(String.self, forKey: .state)) ?? "error"
        message = try? values?.decodeIfPresent(String.self, forKey: .message)
        warnings = try? values?.decodeIfPresent([String].self, forKey: .warnings)
        accent = try? values?.decodeIfPresent(String.self, forKey: .accent)
        action = (try? values?.decode(NativeAction.self, forKey: .action)) ?? NativeAction(kind: "")
        chart = try? values?.decodeIfPresent(NativeWidgetChart.self, forKey: .chart)
        page = try? values?.decodeIfPresent(NativeWidgetPageNode.self, forKey: .page)
    }

    fileprivate static func validateAction(_ action: NativeAction, appId: String) throws {
        guard action.kind == "open_app", action.appId == appId,
              action.eventId == nil, action.runId == nil, action.prompt == nil, action.voice == nil,
              action.operation == nil, action.text == nil, action.files == nil else { throw NativeIntegrationError.invalidAction }
        try NativeAppRoute.validate(path: action.path, queryParams: action.queryParams)
    }

    public func launchURL(scope: String) -> URL {
        (try? NativeWidgetLaunchURL.app(appId: appId, scope: scope, path: action.path,
                                       queryNames: action.queryParams?.map(\.name), queryValues: action.queryParams?.map(\.value))) ?? NativeWidgetLaunchURL.home
    }

    public func isStale(at date: Date) -> Bool {
        NativeSnapshot.date(staleAt).map { $0 <= date } ?? true
    }

    fileprivate func normalized(appIds: Set<String>, now: Date) -> NativeCustomWidget? {
        guard !id.isEmpty, id.utf8.count <= 256, ["chart", "page"].contains(kind),
              appIds.contains(appId), !title.isEmpty, title.utf8.count <= 512,
              let updated = NativeSnapshot.date(updatedAt), let stale = NativeSnapshot.date(staleAt),
              let expiry = NativeSnapshot.date(expiresAt), updated <= now.addingTimeInterval(60),
              stale >= updated, expiry > now, expiry >= stale,
              expiry.timeIntervalSince(updated) <= 7 * 86_400 + 60 else { return nil }
        var result = self
        do {
            try Self.validateAction(action, appId: appId)
            guard ["ready", "empty", "error", "unsupported", "unavailable"].contains(state),
                  accent == nil || ["orange", "blue", "teal", "purple"].contains(accent!),
                  (message?.utf8.count ?? 0) <= 1024, (warnings?.count ?? 0) <= 8,
                  warnings?.allSatisfy({ $0.utf8.count <= 256 }) != false else { throw NativeIntegrationError.invalidAction }
            if state == "ready" {
                if kind == "chart" {
                    guard let chart, page == nil else { throw NativeIntegrationError.invalidAction }
                    try chart.validate()
                } else {
                    guard page != nil, chart == nil else { throw NativeIntegrationError.invalidAction }
                    var nodes = Set<String>(); var images = 0
                    try result.page?.validate(appId: appId, depth: 1, nodes: &nodes, images: &images)
                }
            } else {
                result.chart = nil; result.page = nil
            }
            guard (try JSONEncoder().encode(result)).count <= 131_072 else { throw NativeIntegrationError.oversized }
        } catch {
            result.state = "error"
            result.message = "Open Flow Like to update this widget."
            result.chart = nil; result.page = nil; result.warnings = nil; result.accent = nil
            result.action = NativeAction(kind: "open_app", appId: appId)
        }
        return result
    }

    static func normalize(_ widgets: [NativeCustomWidget]?, apps: [NativeApp], now: Date = Date()) -> [NativeCustomWidget]? {
        guard let widgets else { return nil }
        let appIds = Set(apps.map(\.id))
        var seen = Set<String>()
        var size = 0
        return widgets.prefix(12).compactMap { widget in
            guard seen.insert(widget.id).inserted, let result = widget.normalized(appIds: appIds, now: now),
                  let data = try? JSONEncoder().encode(result), size + data.count <= 1_048_576 else { return nil }
            size += data.count
            return result
        }
    }
}

public struct NativeCustomWidgetSelection: Sendable {
    public enum State: Sendable, Equatable { case ready, chooseWidget, unavailable, expired }
    public let state: State
    public let widget: NativeCustomWidget?

    public init(snapshot: NativeSnapshot?, id: String?, scope: String?, kind: String, now: Date = Date()) {
        guard let id, let scope else { state = .chooseWidget; widget = nil; return }
        guard let snapshot, snapshot.version == 1, snapshot.scope == scope, !scope.isEmpty,
              let current = snapshot.customWidgets?.first(where: { $0.id == id && $0.kind == kind }),
              snapshot.apps.contains(where: { $0.id == current.appId }) else {
            state = .unavailable; widget = nil; return
        }
        guard let expiry = NativeSnapshot.date(current.expiresAt), expiry > now else {
            state = .expired; widget = nil; return
        }
        state = .ready; widget = current
    }
}
