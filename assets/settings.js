(function () {
  "use strict";

  var toggleBtn = document.getElementById("toggle");
  var countEl = document.getElementById("count");
  var hostList = document.getElementById("hostlist");
  var bgStatus = document.getElementById("bg-status");
  var chooseBgBtn = document.getElementById("choose-bg");
  var resetBgBtn = document.getElementById("reset-bg");

  var clearPrivateBtn = document.getElementById("clear-private");
  var privateList = document.getElementById("private-list");
  var privateEmpty = document.getElementById("private-empty");

  var clearHistoryBtn = document.getElementById("clear-history");
  var historySearch = document.getElementById("history-search");
  var historyGroups = document.getElementById("history-groups");
  var historyEmpty = document.getElementById("history-empty");

  var devtoolsToggle = document.getElementById("devtools-toggle");

  var trackingProtectionOn = true;
  var devtoolsOn = false;
  var lastHistory = [];

  function send(cmd) {
    window.ipc.postMessage(JSON.stringify(cmd));
  }

  function applySwitchVisual(btn, enabled) {
    btn.classList.toggle("on", enabled);
    btn.setAttribute("aria-checked", enabled ? "true" : "false");
  }

  toggleBtn.addEventListener("click", function () {
    trackingProtectionOn = !trackingProtectionOn;
    applySwitchVisual(toggleBtn, trackingProtectionOn);
    send({ cmd: "set_tracking_protection", enabled: trackingProtectionOn });
  });

  devtoolsToggle.addEventListener("click", function () {
    devtoolsOn = !devtoolsOn;
    applySwitchVisual(devtoolsToggle, devtoolsOn);
    send({ cmd: "set_devtools", enabled: devtoolsOn });
  });

  chooseBgBtn.addEventListener("click", function () { send({ cmd: "pick_background" }); });
  resetBgBtn.addEventListener("click", function () { send({ cmd: "reset_background" }); });
  clearPrivateBtn.addEventListener("click", function () { send({ cmd: "clear_private_session" }); });
  clearHistoryBtn.addEventListener("click", function () { send({ cmd: "clear_history" }); });

  historySearch.addEventListener("input", function () {
    renderHistory(lastHistory, historySearch.value.trim().toLowerCase());
  });

  // Entries are real <a href> elements: clicking one navigates this tab
  // there through the normal webview navigation path (same policy check
  // as any other link), rather than a separate IPC round trip.
  function makeEntry(entry) {
    var a = document.createElement("a");
    a.className = "entry";
    a.href = entry.url;

    var time = document.createElement("span");
    time.className = "entry-time";
    time.textContent = formatTime(entry.visited_at);
    a.appendChild(time);

    var title = document.createElement("span");
    title.className = "entry-title";
    title.textContent = entry.title || entry.url;
    a.appendChild(title);

    var url = document.createElement("span");
    url.className = "entry-url";
    url.textContent = entry.url;
    a.appendChild(url);

    return a;
  }

  function formatTime(unixSecs) {
    var d = new Date(unixSecs * 1000);
    var h = d.getHours();
    var m = d.getMinutes();
    return (h < 10 ? "0" + h : h) + ":" + (m < 10 ? "0" + m : m);
  }

  function dayLabel(unixSecs) {
    var d = new Date(unixSecs * 1000);
    var now = new Date();
    var oneDay = 86400000;
    var startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    var startOfEntry = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    var diffDays = Math.round((startOfToday - startOfEntry) / oneDay);
    if (diffDays === 0) return "Today";
    if (diffDays === 1) return "Yesterday";
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
  }

  function renderHistory(entries, filter) {
    historyGroups.innerHTML = "";
    var filtered = !filter ? entries : entries.filter(function (e) {
      return (e.url && e.url.toLowerCase().indexOf(filter) !== -1) ||
             (e.title && e.title.toLowerCase().indexOf(filter) !== -1);
    });

    historyEmpty.classList.toggle("hidden", filtered.length !== 0);
    if (filtered.length === 0) return;

    var currentLabel = null;
    var currentList = null;
    for (var i = 0; i < filtered.length; i++) {
      var entry = filtered[i];
      var label = dayLabel(entry.visited_at);
      if (label !== currentLabel) {
        currentLabel = label;
        var group = document.createElement("div");
        group.className = "history-group";
        var h3 = document.createElement("div");
        h3.className = "history-group-label";
        h3.textContent = label;
        group.appendChild(h3);
        currentList = document.createElement("div");
        group.appendChild(currentList);
        historyGroups.appendChild(group);
      }
      currentList.appendChild(makeEntry(entry));
    }
  }

  // Called by Rust (state::sync_settings_pages) after this page loads
  // and after anything on it changes.
  window.scifiSettingsRender = function (state) {
    trackingProtectionOn = !!state.tracking_protection;
    applySwitchVisual(toggleBtn, trackingProtectionOn);
    countEl.textContent = state.blocked_count;

    hostList.innerHTML = "";
    var hosts = state.blocked_hosts || [];
    for (var i = 0; i < hosts.length; i++) {
      var li = document.createElement("li");
      li.textContent = hosts[i];
      hostList.appendChild(li);
    }

    bgStatus.textContent = state.background_set ? "Custom image set" : "Default";
    resetBgBtn.hidden = !state.background_set;

    devtoolsOn = !!state.devtools_enabled;
    applySwitchVisual(devtoolsToggle, devtoolsOn);

    var privateEntries = state.private_session || [];
    privateList.innerHTML = "";
    for (var j = privateEntries.length - 1; j >= 0; j--) {
      privateList.appendChild(makeEntry(privateEntries[j]));
    }
    privateEmpty.classList.toggle("hidden", privateEntries.length !== 0);

    lastHistory = state.history || [];
    renderHistory(lastHistory, historySearch.value.trim().toLowerCase());
  };

  // Same cache-busting refresh hook as chrome.js/home.js.
  window.scifiSetBackground = function (token) {
    document.documentElement.style.setProperty(
      "--scifi-bg-url",
      "url('scifi://asset/background?t=" + token + "')"
    );
  };
})();
