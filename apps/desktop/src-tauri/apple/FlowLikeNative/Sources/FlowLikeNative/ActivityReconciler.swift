import Foundation

struct NativeActivityKey: Hashable {
    var scope: String
    var ownerId: String
    var workId: String
}

struct NativeActivitySelection<Item> {
    var scope: String?
    // A missing list preserves known activities while their source is unavailable.
    var items: [(key: NativeActivityKey, item: Item)]?
}

@MainActor protocol NativeActivityDriver {
    associatedtype Activity
    associatedtype Item

    func activities() -> [Activity]
    func key(for activity: Activity) -> NativeActivityKey
    func selection() -> NativeActivitySelection<Item>
    func observe(_ activity: Activity)
    func update(_ activity: Activity, item: Item) async
    func end(_ activity: Activity) async
    func start(_ item: Item, key: NativeActivityKey)
}

extension NativeActivityDriver {
    func observe(_ activity: Activity) {}
}

@MainActor final class NativeActivityReconciler<Driver: NativeActivityDriver> {
    private let driver: Driver
    private let limit: Int
    private var generation: UInt64 = 0
    private var refreshing = false

    init(_ driver: Driver, limit: Int = 3) {
        self.driver = driver
        self.limit = max(0, limit)
    }

    func refresh() async {
        generation &+= 1
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }

        // Actor isolation permits other refresh requests during an awaited update.
        // Drain their latest selection before allowing another reconciliation to run.
        while true {
            let revision = generation
            let selected = driver.selection()
            await reconcile(selected, generation: revision)
            if isCurrent(revision, scope: selected.scope) { return }
        }
    }

    private func isCurrent(_ revision: UInt64, scope: String?) -> Bool {
        revision == generation && driver.selection().scope == scope
    }

    private func reconcile(_ selected: NativeActivitySelection<Driver.Item>, generation revision: UInt64) async {
        var desired: [(key: NativeActivityKey, item: Driver.Item)] = []
        var desiredKeys: Set<NativeActivityKey> = []
        if let scope = selected.scope, let items = selected.items {
            for entry in items where entry.key.scope == scope {
                guard desired.count < limit else { break }
                if desiredKeys.insert(entry.key).inserted { desired.append(entry) }
            }
        }

        var retained: Set<NativeActivityKey> = []
        for activity in driver.activities() {
            guard isCurrent(revision, scope: selected.scope) else { return }
            let key = driver.key(for: activity)
            let sameScope = selected.scope.map { key.scope == $0 } ?? false
            let wanted = selected.items == nil ? retained.count < limit : desiredKeys.contains(key)
            guard sameScope, wanted, retained.insert(key).inserted else {
                await driver.end(activity)
                continue
            }
            driver.observe(activity)
            if let entry = desired.first(where: { $0.key == key }) {
                await driver.update(activity, item: entry.item)
            }
        }

        guard isCurrent(revision, scope: selected.scope) else { return }
        var existing = Set(driver.activities().map { driver.key(for: $0) })
        for entry in desired where !existing.contains(entry.key) {
            driver.start(entry.item, key: entry.key)
            existing.insert(entry.key)
        }
    }
}
