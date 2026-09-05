# TCS iDRAC Auto-Login (browser extension)

Auto-fills and submits the Dell iDRAC login when you open the console from
Talos Control System, so you get the **real iDRAC HTML5 video console** without
typing credentials.

## Why an extension?

The iDRAC's HTML5 web UI is gated to real browsers (a server-side HTTP client
gets `404`), so TCS's backend cannot proxy the iDRAC console the way it proxies
HPE iLO. But a real browser tab can log into the iDRAC — it's just that a TCS
page is a different origin than the iDRAC, so it can't fill the iDRAC's login
form directly. This tiny extension bridges that gap.

## How it works

1. Click **Console** on a Dell machine in TCS. TCS opens
   `https://<idrac>/login.html#tcs=<token>&m=<machine_id>` in a new tab.
   The `#…` fragment is **never sent to the iDRAC**.
2. The extension's content script reads `#tcs` + `#m`, redeems the **single-use,
   120-second** token against
   `POST {TCS}/api/machines/<machine_id>/console/idrac-autologin/redeem` for this
   machine's iDRAC credentials, fills `#user`/`#password`, and calls the iDRAC's
   own `sendLoginRequest()`.
3. You land logged into the iDRAC; click **Virtual Console** for the video.

The token can only be redeemed once (the nonce is burned server-side) and only
for the machine it was minted for.

## Install (one-time, per machine)

1. Open Chrome → `chrome://extensions`.
2. Enable **Developer mode** (top-right).
3. Click **Load unpacked** → select this `extension/` folder
   (`frontend/extension`).
4. Done. It only acts on iDRAC login pages opened with a TCS fragment.

## Configuration

The TCS base URL is hard-coded in `content.js` as:

```js
const TCS_API = "https://tcs.kronos.cloudmunchers.net/api";
```

If your TCS lives elsewhere, edit that constant and reload the extension.
