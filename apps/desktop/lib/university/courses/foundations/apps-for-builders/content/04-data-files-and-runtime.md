Decide where Equipment Register's resources belong before adding persistence.

| Resource | Home |
| --- | --- |
| A shared equipment handbook | App Storage |
| One user's private draft | User Storage |
| Equipment records that survive runs | A Data Studio table |
| A temporary label during one invocation | A Flow variable |
| A runner-specific endpoint or credential | Runtime configuration |

Create an App Storage folder named `practice` and upload a small synthetic text file containing `Equipment handbook exercise`. Locate it again from the App's Storage workspace. Do not place personal or production files in this exercise.

**Check:** the file lives under the App's Storage, and your plan puts equipment records in durable storage. This lesson does not create a database or prove another user's access; **Data in Flow-Like** and **App Governance** cover those checks.

For a local credential, use **Secret + Runtime Configured**. Such values are excluded from remote execution payloads, so an unattended backend needs its deployment's supported credential mechanism. Before release, follow **Release and Rollback** for tested versions and rollback; the App's version label alone snapshots nothing.

[Storage](https://docs.flow-like.com/apps/storage/) · [Runtime values](https://docs.flow-like.com/apps/runtime-variables/)
