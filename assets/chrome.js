(function () {
  "use strict";

  var tabstrip = document.getElementById("tabstrip");
  var backBtn = document.getElementById("back");
  var fwdBtn = document.getElementById("fwd");
  var reloadBtn = document.getElementById("reload");
  var addressInput = document.getElementById("address");
  var securityDot = document.getElementById("security-dot");
  var settingsBtn = document.getElementById("settings");
  var newIncognitoBtn = document.getElementById("newIncognito");
  var newTabBtn = document.getElementById("newtab");
  var noticeIcon = document.getElementById("notice-icon");

  // Fixed, trusted markup — never built from page-supplied data, so
  // setting these via innerHTML is safe.
  var CLOSE_ICON_SVG =
    '<svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">' +
    '<path d="M3 3l6 6M9 3 3 9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>' +
    "</svg>";
  var INCOGNITO_ICON_SVG =
    '<svg viewBox="0 0 20 20" width="11" height="11" aria-hidden="true">' +
    '<path d="M2.5 7.5h5a2 2 0 1 1 0 4h-5M12.5 7.5h5a2 2 0 1 1 0 4h-5" ' +
    'fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>' +
    '<path d="M7.5 8.8h5" stroke="currentColor" stroke-width="1.6"/>' +
    "</svg>";

  function send(cmd) {
    window.ipc.postMessage(JSON.stringify(cmd));
  }

  backBtn.addEventListener("click", function () { send({ cmd: "go_back" }); });
  fwdBtn.addEventListener("click", function () { send({ cmd: "go_forward" }); });
  reloadBtn.addEventListener("click", function () { send({ cmd: "reload" }); });
  newTabBtn.addEventListener("click", function () { send({ cmd: "new_tab" }); });
  newIncognitoBtn.addEventListener("click", function () { send({ cmd: "new_incognito_tab" }); });
  settingsBtn.addEventListener("click", function () { send({ cmd: "new_tab", url: "scifi://settings/" }); });

  addressInput.addEventListener("keydown", function (e) {
    if (e.key === "Enter") {
      send({ cmd: "navigate", url: addressInput.value });
      addressInput.blur();
    }
  });

  function renderTabs(tabs) {
    tabstrip.innerHTML = "";
    for (var i = 0; i < tabs.length; i++) {
      var tab = tabs[i];
      var pill = document.createElement("div");
      pill.className = "tab" + (tab.is_active ? " active" : "") + (tab.is_incognito ? " incognito" : "");
      pill.setAttribute("role", "tab");

      if (tab.is_incognito) {
        var icon = document.createElement("span");
        icon.className = "tab-icon";
        icon.innerHTML = INCOGNITO_ICON_SVG; // fixed constant, not page-supplied data
        pill.appendChild(icon);
      }

      var label = document.createElement("span");
      label.className = "tab-title";
      label.textContent = tab.title || tab.url; // textContent: never HTML-interpreted
      pill.appendChild(label);

      var close = document.createElement("button");
      close.className = "tab-close";
      close.title = "Close tab";
      close.innerHTML = CLOSE_ICON_SVG;
      close.addEventListener("click", (function (id) {
        return function (e) {
          e.stopPropagation();
          send({ cmd: "close_tab", id: id });
        };
      })(tab.id));
      pill.appendChild(close);

      pill.addEventListener("click", (function (id) {
        return function () { send({ cmd: "switch_tab", id: id }); };
      })(tab.id));

      tabstrip.appendChild(pill);
    }
  }

  // Called by Rust (state::sync_chrome) via evaluate_script after every
  // state change: navigation, tab open/close/switch, or a blocked action.
  window.scifiRender = function (state) {
    renderTabs(state.tabs || []);

    if (document.activeElement !== addressInput) {
      addressInput.value = state.active_url || "";
    }

    backBtn.disabled = !state.can_go_back;
    fwdBtn.disabled = !state.can_go_forward;

    securityDot.className = state.is_secure ? "secure" : "plain";
    securityDot.title = state.is_secure ? "Secure connection" : "Not secure";

    document.body.classList.toggle("incognito", !!state.is_incognito);

    if (state.notice) {
      noticeIcon.hidden = false;
      noticeIcon.title = state.notice;
    } else {
      noticeIcon.hidden = true;
      noticeIcon.title = "";
    }
  };

  // Called by Rust (state::refresh_backgrounds) whenever the saved
  // background image changes, so an already-open page updates without
  // needing a reload. `token` just needs to change to bust the cache.
  window.scifiSetBackground = function (token) {
    document.documentElement.style.setProperty(
      "--scifi-bg-url",
      "url('scifi://asset/background?t=" + token + "')"
    );
  };
})();
