// TCS iDRAC Auto-Login content script.
//
// Runs on Dell iDRAC login pages. When the page URL carries a TCS auto-login
// fragment (e.g. https://<idrac>/login.html#tcs=<token>&m=<machine_id>) — added
// by TCS when you click "Console" on a Dell machine — it redeems the single-use
// token against TCS for this machine's iDRAC credentials, fills the login form,
// and submits it. The fragment is never sent to the iDRAC (URL fragments are not
// transmitted), and the token is burned on first redeem.
//
// TCS base URL — change if your TCS is not at tcs.kronos.cloudmunchers.net.
const TCS_API = "https://tcs.kronos.cloudmunchers.net/api";

(function () {
  "use strict";
  const m = location.hash.match(/#tcs=([^&]+)&m=([0-9a-f-]+)/i);
  if (!m) return; // not opened from TCS — do nothing
  const token = decodeURIComponent(m[1]);
  const machineId = m[2];

  async function redeem() {
    const res = await fetch(
      `${TCS_API}/machines/${machineId}/console/idrac-autologin/redeem`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token }),
      }
    );
    if (!res.ok) {
      console.warn("[TCS iDRAC auto-login] redeem failed:", res.status);
      return null;
    }
    return res.json();
  }

  function fillAndSubmit(creds) {
    const user = document.getElementById("user");
    const pass = document.getElementById("password");
    if (!user || !pass) {
      console.warn("[TCS iDRAC auto-login] login form fields not found");
      return;
    }
    user.value = creds.username;
    pass.value = creds.password;
    // iDRAC login is submitted via its own JS (does a /data/ XHR flow).
    if (typeof window.sendLoginRequest === "function") {
      window.sendLoginRequest();
    } else {
      const form = document.getElementById("auth") || user.closest("form");
      if (form) form.submit();
    }
  }

  // Wait for the form to be present, then redeem + fill.
  const attempt = (left) => {
    if (document.getElementById("password")) {
      redeem()
        .then((c) => c && fillAndSubmit(c))
        .catch((e) => console.warn("[TCS iDRAC auto-login] error", e));
    } else if (left > 0) {
      setTimeout(() => attempt(left - 1), 500);
    }
  };
  attempt(20); // up to ~10s for the iDRAC login page to render its form
})();
