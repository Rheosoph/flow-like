Use an API when a supported API provides the needed operation. This course covers interfaces that require browser or desktop interaction. Practice only on the supplied page or another surface you are authorized to automate.

@PracticeFiles

## Prerequisites

- Flow-Like Desktop with browser automation nodes available.
- Google Chrome or Microsoft Edge (on macOS and Linux, Chromium also works). No WebDriver or chromedriver is needed: Open Browser launches the browser directly. Without one, install Google Chrome, or install Chrome for Testing in **Settings > Automation**. On Ubuntu 23.10 and newer, use Google Chrome, Edge or Ubuntu's Snap Chromium instead of Chrome for Testing, because AppArmor blocks its sandbox there.
- Python 3. In the unzipped fixture directory run `python3 -m http.server 8766 --bind 127.0.0.1`.

Open `http://127.0.0.1:8766/portal.html` manually. It should display PO-001 and, after a short delay, `Ready to ship`. This page contains no credentials or external submission.

Your flow will start an automation session, open a browser, locate a stable element, read it, then close all resources. Browser nodes need a compatible local execution environment; selecting Remote does not provision a browser.

Keep the page server running until the lab ends. If the page works manually but Open Browser fails, read its error in the run log. A missing browser is reported with every location searched: install that browser, or set Browser Type to the browser you have.
