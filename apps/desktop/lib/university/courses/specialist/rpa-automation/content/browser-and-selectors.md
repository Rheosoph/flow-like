Build a local scratch flow. Pass each node's Session output to the next node's Session input as well as connecting execution.

Open Browser launches Chrome or Edge directly, so no WebDriver or chromedriver is needed. Set Browser Type to the browser you have installed (Chrome or Edge); on macOS and Linux, Browser Type Chrome also uses Chromium. With neither Chrome nor Chromium installed, install Google Chrome, or open **Settings > Automation** in Flow-Like Desktop and install Chrome for Testing (about 200 MB). On Ubuntu 23.10 and newer, skip Chrome for Testing: AppArmor blocks its sandbox, and it would also be picked before Ubuntu's Snap Chromium, which otherwise works. Use Google Chrome, Snap Chromium, or Edge with Browser Type Edge there. Firefox and Safari are not supported yet.

1. **Simple Event → Start Automation Session → Open Browser**. Set Browser Type to your installed browser and Headless to false for inspection. Leave WebDriver URL at its default; a local address there is ignored.
2. **New Page → Go To URL**, using `http://127.0.0.1:8766/portal.html`.
3. Add **Wait For Selector**. Selector: `#status[data-ready="true"]`; Timeout: 5000 ms.
4. Branch on its **Found** output. On true, call **Get Text** with selector `#status`, then Print Info with Text. On false, report `practice status unavailable` and skip extraction.
5. Close Page, Close Browser and Stop Automation Session after the work. Arrange cleanup on caught failure paths as well; never leave cleanup reachable only after a successful extraction.

Expected text: `Ready to ship`. The data-ready attribute indicates the current load finished. Waiting for `#status` alone would find the initial Loading cell immediately.

Now use `/portal.html?banner=1`. The maintenance banner moves the table but the selector should still find the same element. Record the output from both layouts.

Use stable IDs or meaningful attributes rather than a position such as the third cell on the screen. Coordinates are specific to a display arrangement and do not follow the element when layout changes.

To drive a browser that is already running, use **Attach to Browser** instead of Open Browser. For a Chrome or Edge started with `--remote-debugging-port` and its own `--user-data-dir`, enter `127.0.0.1:<port>` as Debugger Address; a `wss://` DevTools WebSocket URL is for a browser service on another machine. To use your everyday Chrome 144 or newer, open `chrome://inspect/#remote-debugging`, turn on **Allow remote debugging for this browser instance**, clear Debugger Address, and click **Allow** when Chrome asks during the run. For Edge, also set the node's Browser input to Edge. That gives the flow your profile's cookies and logins, so keep this lab on Open Browser.
