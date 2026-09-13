import Foundation

struct NativeGeofenceLeaseState {
    private(set) var token: UUID?
    private(set) var scope: String?
    private(set) var deadline: Date?

    mutating func begin(scope: String?, deadline: Date) -> UUID {
        let token = UUID()
        self.token = token
        self.scope = scope
        self.deadline = deadline
        return token
    }

    mutating func finish(expectedToken: UUID? = nil) -> Bool {
        if let expectedToken, expectedToken != token { return false }
        token = nil
        scope = nil
        deadline = nil
        return true
    }
}
