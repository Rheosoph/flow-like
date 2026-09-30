Build a local scratch flow. Pass each node's Session output to the next node's Session input as well as connecting execution.

1. **Simple Event → Start Automation Session → Open Browser**. Set WebDriver URL to `http://localhost:9515`, Browser Type to Chrome and Headless to false for inspection.
2. **New Page → Go To URL**, using `http://127.0.0.1:8766/portal.html`.
3. Add **Wait For Selector**. Selector: `#status[data-ready="true"]`; Timeout: 5000 ms.
4. Branch on its **Found** output. On true, call **Get Text** with selector `#status`, then Print Info with Text. On false, report `practice status unavailable` and skip extraction.
5. Close Page, Close Browser and Stop Automation Session after the work. Arrange cleanup on caught failure paths as well; never leave cleanup reachable only after a successful extraction.

Expected text: `Ready to ship`. The data-ready attribute indicates the current load finished. Waiting for `#status` alone would find the initial Loading cell immediately.

Now use `/portal.html?banner=1`. The maintenance banner moves the table but the selector should still find the same element. Record the output from both layouts.

Use stable IDs or meaningful attributes rather than a position such as the third cell on the screen. Coordinates are specific to a display arrangement and do not follow the element when layout changes.
