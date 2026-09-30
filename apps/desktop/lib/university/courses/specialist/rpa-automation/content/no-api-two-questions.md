Use an API when a supported API provides the needed operation. This course covers interfaces that require browser or desktop interaction. Practice only on the supplied page or another surface you are authorized to automate.

@PracticeFiles

## Prerequisites

- Flow-Like Desktop with browser automation nodes available.
- A matching browser and WebDriver service. For ChromeDriver, start the installed driver with `chromedriver --port=9515`. Its version must support your Chrome installation. Use the URL and browser type for your own driver if different.
- Python 3. In the unzipped fixture directory run `python3 -m http.server 8766 --bind 127.0.0.1`.

Open `http://127.0.0.1:8766/portal.html` manually. It should display PO-001 and, after a short delay, `Ready to ship`. This page contains no credentials or external submission.

Your flow will start an automation session, open a browser, locate a stable element, read it, then close all resources. Browser nodes need a compatible local execution environment; selecting Remote does not provision a browser or WebDriver.

Keep both local services running until the lab ends. If the page works manually but the flow cannot connect, check the WebDriver service separately from the page server.
