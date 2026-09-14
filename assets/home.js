(function () {
  "use strict";
  // Same cache-busting refresh hook as chrome.js — see there for why
  // this exists. The home page has no other Rust <-> JS traffic.
  window.scifiSetBackground = function (token) {
    document.documentElement.style.setProperty(
      "--scifi-bg-url",
      "url('scifi://asset/background?t=" + token + "')"
    );
  };
})();
