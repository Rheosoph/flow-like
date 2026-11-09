import Foundation
import Testing
@testable import FlowLikeNative

@MainActor private final class ActivityUpdateGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var entered: [CheckedContinuation<Void, Never>] = []

    func suspend() async {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
            let waiting = entered
            entered.removeAll()
            waiting.forEach { $0.resume() }
        }
    }

    func waitUntilSuspended() async {
        guard continuation == nil else { return }
        await withCheckedContinuation { entered.append($0) }
    }

    func resume() {
        let waiting = continuation
        continuation = nil
        waiting?.resume()
    }
}

@MainActor private final class ActivityTestDriver: NativeActivityDriver {
    struct Entry {
        var id: String
        var key: NativeActivityKey
        var status: String
    }

    var entries: [Entry] = []
    var selected = NativeActivitySelection<String>(scope: "alice", items: [])
    var updateGate: ActivityUpdateGate?
    var endGate: ActivityUpdateGate?
    var observed: [String] = []
    var ended: [String] = []
    var started: [NativeActivityKey] = []
    var updating = 0
    var maximumConcurrentUpdates = 0

    func activities() -> [Entry] { entries }
    func key(for activity: Entry) -> NativeActivityKey { activity.key }
    func selection() -> NativeActivitySelection<String> { selected }
    func observe(_ activity: Entry) { observed.append(activity.id) }

    func update(_ activity: Entry, item: String) async {
        updating += 1
        maximumConcurrentUpdates = max(maximumConcurrentUpdates, updating)
        if let gate = updateGate {
            updateGate = nil
            await gate.suspend()
        }
        if let index = entries.firstIndex(where: { $0.id == activity.id }) {
            entries[index].status = item
        }
        updating -= 1
    }

    func end(_ activity: Entry) async {
        if let gate = endGate {
            endGate = nil
            await gate.suspend()
        }
        ended.append(activity.id)
        entries.removeAll { $0.id == activity.id }
    }

    func start(_ item: String, key: NativeActivityKey) {
        started.append(key)
        entries.append(Entry(id: UUID().uuidString, key: key, status: item))
    }

    func select(_ work: [String], scope: String = "alice", status: String = "running") {
        selected = NativeActivitySelection(scope: scope, items: work.map {
            (key: activityKey($0, scope: scope), item: status)
        })
    }

    func add(_ work: String, id: String? = nil, scope: String = "alice", status: String = "queued") {
        entries.append(Entry(id: id ?? work, key: activityKey(work, scope: scope), status: status))
    }
}

private func activityKey(_ work: String, scope: String = "alice", owner: String = "bot") -> NativeActivityKey {
    NativeActivityKey(scope: scope, ownerId: owner, workId: work)
}

@Test @MainActor func overlappingActivityRefreshesCannotStartTheSameTurnTwice() async {
    let driver = ActivityTestDriver()
    driver.add("existing")
    driver.select(["existing", "new"])
    let gate = ActivityUpdateGate()
    driver.updateGate = gate
    let reconciler = NativeActivityReconciler(driver)
    let first = Task { await reconciler.refresh() }
    await gate.waitUntilSuspended()

    await reconciler.refresh()
    #expect(driver.maximumConcurrentUpdates == 1)
    #expect(driver.started.isEmpty)
    gate.resume()
    await first.value

    #expect(driver.entries.filter { $0.key == activityKey("new") }.count == 1)
    #expect(driver.started == [activityKey("new")])
    #expect(driver.entries.first { $0.key == activityKey("existing") }?.status == "running")
    #expect(driver.maximumConcurrentUpdates == 1)
}

@Test @MainActor func activityRefreshesCoalesceToTheLatestTurnSelection() async {
    let driver = ActivityTestDriver()
    driver.add("existing")
    driver.select(["existing", "obsolete"])
    let gate = ActivityUpdateGate()
    driver.updateGate = gate
    let reconciler = NativeActivityReconciler(driver)
    let first = Task { await reconciler.refresh() }
    await gate.waitUntilSuspended()

    driver.select(["existing", "intermediate"])
    await reconciler.refresh()
    driver.select(["existing", "latest"], status: "waiting")
    await reconciler.refresh()
    gate.resume()
    await first.value

    #expect(driver.started == [activityKey("latest")])
    #expect(driver.entries.map(\.status) == ["waiting", "waiting"])
}

@Test @MainActor func activitySelectionDeduplicatesBeforeApplyingTheLimit() async {
    let driver = ActivityTestDriver()
    driver.select(["first", "first", "second", "third", "fourth"])
    await NativeActivityReconciler(driver).refresh()
    #expect(driver.started.map(\.workId) == ["first", "second", "third"])
}

@Test @MainActor func activityReorderingRetiresCardsOutsideTheLimit() async {
    let driver = ActivityTestDriver()
    let reconciler = NativeActivityReconciler(driver)
    driver.select(["first", "second", "third", "fourth"])
    await reconciler.refresh()
    let thirdID = driver.entries.first { $0.key.workId == "third" }?.id

    driver.select(["fourth", "first", "second", "third"])
    await reconciler.refresh()

    #expect(driver.entries.count == 3)
    #expect(Set(driver.entries.map { $0.key.workId }) == ["first", "second", "fourth"])
    #expect(driver.ended.first == thirdID)
}

@Test @MainActor func activityRefreshRemovesExistingDuplicatesAndCompletedTurns() async {
    let driver = ActivityTestDriver()
    driver.add("active", id: "kept")
    driver.add("active", id: "duplicate")
    driver.add("completed")
    driver.select(["active"])
    await NativeActivityReconciler(driver).refresh()

    #expect(driver.entries.map(\.id) == ["kept"])
    #expect(driver.ended == ["duplicate", "completed"])
    #expect(driver.observed == ["kept"])
    #expect(driver.entries.first?.status == "running")
}

@Test @MainActor func unavailableActivityDataPreservesOnlyUniqueCurrentScopeCardsWithinTheLimit() async {
    let driver = ActivityTestDriver()
    driver.add("first")
    driver.add("first", id: "duplicate")
    driver.add("second")
    driver.add("third")
    driver.add("fourth")
    driver.add("foreign", scope: "bob")
    driver.selected = NativeActivitySelection(scope: "alice", items: nil)
    await NativeActivityReconciler(driver).refresh()

    #expect(driver.entries.map(\.id) == ["first", "second", "third"])
    #expect(driver.ended == ["duplicate", "fourth", "foreign"])
    #expect(driver.observed == ["first", "second", "third"])
    #expect(driver.entries.allSatisfy { $0.status == "queued" })
    #expect(driver.started.isEmpty)
}

@Test @MainActor func activityScopeChangesDuringAnUpdateCannotRecreateThePreviousAccountsCards() async {
    let driver = ActivityTestDriver()
    driver.add("existing")
    driver.select(["existing", "private"])
    let gate = ActivityUpdateGate()
    driver.updateGate = gate
    let reconciler = NativeActivityReconciler(driver)
    let refresh = Task { await reconciler.refresh() }
    await gate.waitUntilSuspended()

    driver.select(["other"], scope: "bob")
    gate.resume()
    await refresh.value

    #expect(driver.ended == ["existing"])
    #expect(driver.started == [activityKey("other", scope: "bob")])
    #expect(driver.entries.allSatisfy { $0.key.scope == "bob" })
}

@Test @MainActor func signOutDuringAnActivityUpdateClearsEveryCard() async {
    let driver = ActivityTestDriver()
    driver.add("existing")
    driver.select(["existing", "new"])
    let gate = ActivityUpdateGate()
    driver.updateGate = gate
    let reconciler = NativeActivityReconciler(driver)
    let refresh = Task { await reconciler.refresh() }
    await gate.waitUntilSuspended()

    driver.selected = NativeActivitySelection(scope: nil, items: nil)
    await reconciler.refresh()
    gate.resume()
    await refresh.value

    #expect(driver.entries.isEmpty)
    #expect(driver.ended == ["existing"])
    #expect(driver.started.isEmpty)
}

@Test @MainActor func activityScopeChangesDuringRemovalUseTheLatestAccount() async {
    let driver = ActivityTestDriver()
    driver.add("completed")
    driver.select(["obsolete"])
    let gate = ActivityUpdateGate()
    driver.endGate = gate
    let reconciler = NativeActivityReconciler(driver)
    let refresh = Task { await reconciler.refresh() }
    await gate.waitUntilSuspended()

    driver.select(["latest"], scope: "bob")
    gate.resume()
    await refresh.value

    #expect(driver.ended == ["completed"])
    #expect(driver.started == [activityKey("latest", scope: "bob")])
}

@Test @MainActor func activityKeysKeepOwnersSeparateAndRejectForeignScopeSelections() async {
    let driver = ActivityTestDriver()
    let first = activityKey("shared", owner: "first")
    let second = activityKey("shared", owner: "second")
    driver.selected = NativeActivitySelection(scope: "alice", items: [
        (key: activityKey("foreign", scope: "bob"), item: "queued"),
        (key: first, item: "running"),
        (key: second, item: "waiting"),
    ])
    await NativeActivityReconciler(driver).refresh()
    #expect(driver.started == [first, second])
}

@Test @MainActor func anEmptyConfirmedActivitySelectionRemovesAllCards() async {
    let driver = ActivityTestDriver()
    driver.add("first")
    driver.add("second")
    driver.select([])
    await NativeActivityReconciler(driver).refresh()
    #expect(driver.entries.isEmpty)
    #expect(driver.ended == ["first", "second"])
}
