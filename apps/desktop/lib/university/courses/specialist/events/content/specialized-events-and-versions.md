Choose one integration you already have permission and test credentials to configure. This class is a configuration exercise; it does not require accounts with every provider.

| Surface | Configuration to inspect | Test evidence |
| --- | --- | --- |
| Chat UI | Chat Event node and route | A synthetic message reaches the flow |
| Discord or Telegram | Bot credential, allowed channel/chat, local adapter or supported hosted delivery | One allowed message invokes one run |
| Teams | Supported server, bot connection, Chat Event | A message or card action reaches the expected handler |
| Email | Mail Event, IMAP connection and mailbox | One test mail reaches the flow |
| Inbound Email | Server support and generated address | A test message appears in the inbound Event context |
| Geolocation | Device permission, region and enter/leave trigger | A permitted device transition invokes the Event |

Keep the test flow read-only: log a synthetic message ID or a count, without replying or writing to a business system. Restrict a bot to your test chat. For mail, distinguish a local IMAP adapter from a generated inbound address; they use different nodes and deployment requirements.

Local Telegram polling and a registered webhook compete for delivery. Use the selected configuration mode and its setup controls; test only one delivery path for the bot token at a time. Server availability and credentials are deployment prerequisites, not things a course quiz can establish.

Record the chosen input, active adapter, runtime location and run evidence. If the integration is unavailable on your hub, keep this class optional and use Chat UI for the exercise.

Use **Release and Rollback** before exposing a versioned flow to other users.
