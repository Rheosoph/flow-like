---
title: Offline vs. Online
description: Offline and Online Apps Explained
sidebar:
  order: 10
---

When creating apps in Flow-Like, you can choose to make them either **offline** or **online**.

Unlike offline apps, **online apps** can be shared with other users, published in the **Flow-Like App Store**, and accessed from multiple devices. Regardless of the mode, you can always export any app and re-import it later if you prefer not to go online.

### Why Choose Offline or Online at App Creation?
Currently, it is not yet possible to convert an offline app into an online app after creation. However, this feature is planned. You can track our progress in [this issue](https://github.com/Rheosoph/flow-like/issues/280).

### Where Does My Flow Actually Run?

It depends on how you're accessing your app and how the flow is configured:

| Access Method | Execution Location |
|---------------|-------------------|
| **Web App** | Always runs on the server (cloud or self-hosted) |
| **Desktop App (offline app)** | Always runs locally on your machine |
| **Desktop App (online app, default)** | Runs locally on your machine |
| **Desktop App (online-only board/event)** | Runs on the server |
| **Self-hosted deployment** | Runs in a dedicated execution container on your server |

#### Hybrid Execution

Online apps support **hybrid execution**—the same flow can run in different locations depending on how it's accessed:

- **From Flow-Like Desktop** → Executes locally on your machine (unless marked as online-only)
- **From the Web App** → Executes on the server

This means you can test and develop flows locally, then access them from anywhere via the web.

#### Online-Only Boards & Events

You can mark specific boards or events as **online-only**, which forces them to always execute on the server—even when accessed from the Desktop app. This is useful for:

- Flows that need to run 24/7 without your computer being on
- Scheduled automations and webhooks
- Flows that should run in a consistent server environment

#### Self-Hosting

When [self-hosting Flow-Like](/self-hosting/overview/), all server-side execution happens in a dedicated container on your infrastructure. You maintain full control over where your data and flows run.

:::note
Please make sure you are using the same version of Flow-Like on all devices from which you want to access your online apps.
:::

### Board Execution Mode

Each board has an **execution mode** setting that controls where it runs:

| Execution Mode | Desktop App | Web App |
|---------------|-------------|---------|
| **Hybrid** (default) | Runs locally | Runs on server |
| **Local** | Runs locally | Cannot execute |
| **Remote** | Runs on server | Runs on server |

You can set the execution mode in the board settings. Use **Remote** when you want the flow to always run on the server (e.g., for scheduled tasks), and **Local** for flows that must never leave your machine.

### Permission-Based Execution

When sharing apps with other users, you can grant different permission levels:

| Permission | Can Execute Locally | Can Execute on Server |
|------------|---------------------|----------------------|
| **Execute + Read Boards** | ✅ Yes | ✅ Yes |
| **Execute Only** | ❌ No | ✅ Yes |

Users with only **Execute** permission (without **Read Boards**) can only run flows on the server. This is useful when you want to share an app's functionality without giving access to the underlying flow logic.

### Local-Only Execution

Some flows **must always run locally** on your device, regardless of whether your app is offline or online. This includes:

- **RPA (Robotic Process Automation)** – Controlling your desktop, mouse, and keyboard
- **Browser automation** – Interacting with a real browser window
- **Local file access** – Reading and writing files on your computer
- **Hardware integration** – Accessing cameras, microphones, or other devices

Learn more about which nodes require local execution in our [Local-Only Execution guide](/studio/local-execution/).