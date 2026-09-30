Use a dedicated IMAP test mailbox containing only the two synthetic fixture messages. Import them through your mail client or send equivalent test messages to that account. Configure credentials through runtime secrets.

1. Connect **IMAP Connect → IMAP Inbox → List Mails**. Set List Mails Filter to `UNSEEN`.
2. Iterate the references with **For Each** and read them with **Fetch Mail**. Use **Email → Headers** and **Email → Content** to inspect sender, subject and text.
3. Route an `Auto-Submitted` automated reply to a no-draft branch. For the practice question, use **Create Draft** to write one response in the test account's Drafts mailbox. Keep the synthetic recipient and do not send it.
4. Only after the intended draft or filing succeeds, use **Mark Mail as Seen** or **Move Mail to Mailbox** to remove that message from the swept set.
5. Inspect Drafts and the source flag/folder. Run the sweep again. With this successful sequential run, expect no second draft.

Now consider a crash after Create Draft but before Mark Mail as Seen. The message can reappear next sweep. For a production design, use the mailbox's stable identity, including UID validity/UID where applicable, and record/reconcile the created draft. Concurrent sweeps also need a claim or serialization mechanism. `UNSEEN` by itself is not an exactly-once guarantee.

Keep this lab manual. Add a schedule only after you can explain partial failure and repeated execution. Never auto-answer automated mail.
