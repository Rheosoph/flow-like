import Foundation
import UserNotifications

public enum NativeBotNotifications {
    public static let messageCategory = "flow_like_bot_message"
    public static let approvalCategory = "flow_like_bot_approval"
    public static let reviewCategory = "flow_like_bot_review"
    public static let replyAction = "flow_like_bot_reply"
    public static let approveOnceAction = "flow_like_bot_approve_once"
    public static let denyAction = "flow_like_bot_deny"

    public static func registerCategories(center: UNUserNotificationCenter = .current()) {
        let reply = UNTextInputNotificationAction(identifier: replyAction, title: "Reply",
            options: [.authenticationRequired, .foreground], textInputButtonTitle: "Send", textInputPlaceholder: "Message the bot")
        let approve = UNNotificationAction(identifier: approveOnceAction, title: "Allow once", options: [.authenticationRequired, .foreground])
        let deny = UNNotificationAction(identifier: denyAction, title: "Deny", options: [.authenticationRequired, .foreground, .destructive])
        let categories: Set<UNNotificationCategory> = [
            UNNotificationCategory(identifier: messageCategory, actions: [reply], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: approvalCategory, actions: [approve, deny], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: reviewCategory, actions: [], intentIdentifiers: [], options: []),
        ]
        center.getNotificationCategories { existing in
            center.setNotificationCategories(Set(existing.filter { ![messageCategory, approvalCategory, reviewCategory].contains($0.identifier) }).union(categories))
        }
    }

    /// The host rechecks account access and the current approval before executing this queued action.
    @available(iOSApplicationExtension, unavailable)
    public static func handleResponse(_ response: UNNotificationResponse, store: NativeStore = .shared) -> Bool {
        handle(category: response.notification.request.content.categoryIdentifier, identifier: response.actionIdentifier,
            userInfo: response.notification.request.content.userInfo,
            reply: (response as? UNTextInputNotificationResponse)?.userText, store: store)
    }

    @available(iOSApplicationExtension, unavailable)
    static func handle(category: String, identifier: String, userInfo: [AnyHashable: Any], reply: String?, store: NativeStore) -> Bool {
        guard [messageCategory, approvalCategory, reviewCategory].contains(category) else { return false }
        if identifier == UNNotificationDismissActionIdentifier { return true }
        let isOpen = identifier == UNNotificationDefaultActionIdentifier
        let expectedCategory = identifier == replyAction ? messageCategory : approvalCategory
        guard isOpen || category == expectedCategory else { return true }
        guard let snapshot = store.catalog(),
              let action = action(identifier: identifier, userInfo: userInfo, reply: reply, snapshot: snapshot) else {
            if isOpen { _ = try? store.enqueue(NativeAction(kind: "open_home")) }
            return true
        }
        let bot = snapshot.bots?.first(where: { $0.id == action.botId })
        let background = !isOpen && bot?.local == false && bot?.encrypted == false &&
            userInfo["conversation_encryption"] as? String == "plain"
        _ = try? store.enqueue(action, expectedScope: snapshot.scope, backgroundDelivery: background)
        #if os(iOS)
        Task { @MainActor in NativeBotBackgroundRefresh.schedule() }
        #endif
        // Recognized bot payloads never fall through to an unscoped generic tap store.
        return true
    }

    static func action(identifier: String, userInfo: [AnyHashable: Any], reply: String?,
                       snapshot: NativeSnapshot, now: Date = Date()) -> NativeAction? {
        func field(_ name: String) -> String? {
            guard let value = userInfo[name] as? String, !value.isEmpty, value.utf8.count <= 512,
                  !value.unicodeScalars.contains(where: { $0.properties.generalCategory == .control }) else { return nil }
            return value
        }
        guard let userId = field("user_id"), userId == snapshot.userId,
              let scope = userInfo["native_scope"] as? String, scope == snapshot.scope,
              let botId = field("bot_id"), snapshot.bots?.contains(where: { $0.id == botId && $0.enabled != false }) == true else { return nil }
        if let expires = userInfo["expires_at"] {
            guard let value = expires as? String, let date = NativeSnapshot.date(value), date > now else { return nil }
        }
        let conversationId = field("conversation_id")
        if identifier == UNNotificationDefaultActionIdentifier {
            return NativeAction(kind: conversationId == nil ? "open_bot" : "open_conversation", botId: botId,
                conversationId: conversationId, turnId: field("turn_id"), approvalId: field("approval_id"))
        }
        if identifier == replyAction {
            guard let prompt = reply?.trimmingCharacters(in: .whitespacesAndNewlines), !prompt.isEmpty,
                  prompt.utf8.count <= 65_536 else { return nil }
            return NativeAction(kind: "ask_bot", prompt: prompt, botId: botId, conversationId: conversationId)
        }
        guard [approveOnceAction, denyAction].contains(identifier),
              let approvalId = field("approval_id"), let turnId = field("turn_id") else { return nil }
        let action = NativeAction(kind: "respond_bot_approval", botId: botId, conversationId: conversationId,
            turnId: turnId, approvalId: approvalId, decision: identifier == approveOnceAction ? "once" : "deny")
        guard (try? snapshot.validateBotAction(action, now: now)) != nil else { return nil }
        return action
    }
}
