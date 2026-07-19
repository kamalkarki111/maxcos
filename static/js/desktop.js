
(function () {
  const M = window.MAXCOS || {};
  const { createIcon } = window.MaxcosIcons;
  const { openApp } = window.MaxcosApps;
  const WM = window.MaxcosWM;

  // Init spaces from server settings
  const settings = M.settings || { spaces: [], active_space: 0, wallpaper: "sonoma" };
  if (!settings.spaces || !settings.spaces.length) {
    settings.spaces = [
      { id: "space-1", name: "Desktop 1", wallpaper: "sonoma" },
      { id: "space-2", name: "Desktop 2", wallpaper: "sequoia" },
    ];
    settings.active_space = 0;
  }
  const activeId = settings.spaces[settings.active_space]?.id || settings.spaces[0].id;
  WM.setSpaces(settings.spaces, activeId);

  function esc(s) {
    return String(s ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function toast(title, body) {
    const stack = document.getElementById("toast-stack");
    const el = document.createElement("div");
    el.className = "toast";
    el.innerHTML = `<strong>${esc(title)}</strong><p>${esc(body)}</p>`;
    stack.appendChild(el);
    setTimeout(() => {
      el.style.opacity = "0";
      el.style.transition = "opacity .3s";
      setTimeout(() => el.remove(), 300);
    }, 3200);
  }

  async function persistSettings() {
    const spaces = WM.getSpaces();
    const active = WM.getActiveSpaceId();
    const idx = Math.max(0, spaces.findIndex((s) => s.id === active));
    const payload = {
      wallpaper: spaces[idx]?.wallpaper || settings.wallpaper || "sonoma",
      spaces,
      active_space: idx,
    };
    try {
      await fetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      M.settings = payload;
    } catch (_) {}
  }

  async function updateClock() {
    try {
      const d = await (await fetch("/api/time")).json();
      const el = document.getElementById("menubar-clock");
      if (el) el.textContent = `${d.date}  ${d.time}`;
    } catch (_) {}
  }
  setInterval(updateClock, 15000);

  function buildDock() {
    const inner = document.getElementById("dock-inner");
    inner.innerHTML = "";
    (M.dockApps || []).forEach((app) => {
      if (app.id === "trash") {
        const sep = document.createElement("div");
        sep.className = "dock-separator";
        inner.appendChild(sep);
      }
      const item = document.createElement("button");
      item.className = "dock-item";
      item.dataset.app = app.id;
      const tip = document.createElement("span");
      tip.className = "dock-tooltip";
      tip.textContent = app.name;
      item.appendChild(tip);
      item.appendChild(createIcon(app.icon || app.id));
      const dot = document.createElement("span");
      dot.className = "dot";
      item.appendChild(dot);
      item.onclick = () => {
        if (app.id === "launchpad") {
          toggleLaunchpad();
          return;
        }
        const existing = WM.findWindowByApp(app.id);
        if (existing) {
          existing.classList.remove("minimized");
          // switch to window's space
          const w = [...WM.windows.values()].find((x) => x.el === existing);
          if (w) WM.switchSpace(w.spaceId);
          WM.focusWindow(existing);
        } else openApp(app.id);
      };
      inner.appendChild(item);
    });
  }

  function buildDesktopIcons() {
    const c = document.getElementById("desktop-icons");
    c.innerHTML = "";
    (M.desktopIcons || []).forEach((app) => {
      const el = document.createElement("button");
      el.className = "desk-icon";
      el.appendChild(createIcon(app.icon || app.id, 52));
      const s = document.createElement("span");
      s.textContent = app.name;
      el.appendChild(s);
      el.onclick = () => {
        c.querySelectorAll(".desk-icon").forEach((i) => i.classList.remove("selected"));
        el.classList.add("selected");
      };
      el.ondblclick = () => openApp(app.id);
      c.appendChild(el);
    });
  }

  function toggleLaunchpad(force) {
    const lp = document.getElementById("launchpad");
    const show = force !== undefined ? force : lp.hidden;
    lp.hidden = !show;
    if (show) {
      const grid = document.getElementById("launchpad-grid");
      grid.innerHTML = "";
      (M.allApps || [])
        .filter((a) => a.id !== "trash" && a.id !== "launchpad")
        .forEach((app) => {
          const item = document.createElement("button");
          item.className = "lp-item";
          item.appendChild(createIcon(app.icon || app.id, 64));
          const sp = document.createElement("span");
          sp.textContent = app.name;
          item.appendChild(sp);
          item.onclick = () => {
            toggleLaunchpad(false);
            openApp(app.id);
          };
          grid.appendChild(item);
        });
    }
  }
  document.getElementById("launchpad")?.addEventListener("click", (e) => {
    if (e.target.id === "launchpad") toggleLaunchpad(false);
  });

  // Apple menu
  const appleBtn = document.getElementById("apple-btn");
  const appleDrop = document.getElementById("apple-dropdown");
  appleBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = !appleDrop.hidden;
    closeMenus();
    appleDrop.hidden = open;
    appleBtn.classList.toggle("open", !open);
  });
  appleDrop?.querySelectorAll("button").forEach((btn) => {
    btn.onclick = () => {
      const act = btn.dataset.action;
      appleDrop.hidden = true;
      appleBtn.classList.remove("open");
      if (act === "about") document.getElementById("about-modal").hidden = false;
      else if (act === "settings") openApp("systemsettings");
      else if (act === "appstore") openApp("appstore");
      else if (act === "mission") toggleMissionControl(true);
      else if (act === "switch" || act === "logout") {
        const f = document.createElement("form");
        f.method = "POST";
        f.action = "/logout";
        document.body.appendChild(f);
        f.submit();
      }
    };
  });
  document.getElementById("about-close")?.addEventListener("click", (e) => {
    e.preventDefault();
    document.getElementById("about-modal").hidden = true;
  });
  document.getElementById("about-modal")?.addEventListener("click", (e) => {
    if (e.target.id === "about-modal") e.target.hidden = true;
  });

  const cc = document.getElementById("control-center");
  document.getElementById("cc-btn")?.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = !cc.hidden;
    closeMenus();
    cc.hidden = open;
  });
  document.getElementById("brightness")?.addEventListener("input", (e) => {
    document.getElementById("wallpaper").style.filter = `brightness(${0.5 + (e.target.value / 100) * 0.5})`;
  });

  // Notifications
  const notifCenter = document.getElementById("notif-center");
  const notifBadge = document.getElementById("notif-badge");
  async function refreshNotifications() {
    try {
      const data = await (await fetch("/api/notifications")).json();
      const unread = data.unread || 0;
      if (notifBadge) {
        if (unread > 0) {
          notifBadge.hidden = false;
          notifBadge.textContent = unread > 99 ? "99+" : String(unread);
        } else notifBadge.hidden = true;
      }
      const list = document.getElementById("notif-list");
      if (!list) return;
      const items = data.notifications || [];
      if (!items.length) {
        list.innerHTML = `<div class="notif-empty">No notifications</div>`;
        return;
      }
      list.innerHTML = items
        .map(
          (n) => `<button type="button" class="notif-card${n.read ? "" : " unread"}" data-id="${n.id}" data-app="${esc(n.app_id || "")}">
        <div class="n-app"><span>${esc(n.app)}</span><span>${esc(n.time)}</span></div>
        <div class="n-title">${esc(n.title)}</div><div class="n-body">${esc(n.body)}</div></button>`
        )
        .join("");
      list.querySelectorAll(".notif-card").forEach((card) => {
        card.onclick = async () => {
          await fetch("/api/notifications/" + card.dataset.id + "/read", { method: "POST" });
          if (card.dataset.app) openApp(card.dataset.app);
          refreshNotifications();
        };
      });
    } catch (_) {}
  }
  function toggleNotifCenter(show) {
    const open = show !== undefined ? show : notifCenter.hidden;
    closeMenus();
    notifCenter.hidden = !open;
    if (open) refreshNotifications();
  }
  document.getElementById("notif-btn")?.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleNotifCenter();
  });
  document.getElementById("notif-read-all")?.addEventListener("click", async (e) => {
    e.stopPropagation();
    await fetch("/api/notifications/read-all", { method: "POST" });
    refreshNotifications();
  });
  document.getElementById("notif-clear")?.addEventListener("click", async (e) => {
    e.stopPropagation();
    await fetch("/api/notifications/clear", { method: "POST" });
    refreshNotifications();
  });

  // Spotlight
  const spotlight = document.getElementById("spotlight");
  const spotInput = document.getElementById("spotlight-input");
  const spotResults = document.getElementById("spotlight-results");
  let spotHits = [],
    spotIndex = 0,
    spotTimer = null;
  function toggleSpotlight(show) {
    const open = show !== undefined ? show : spotlight.hidden;
    closeMenus();
    if (notifCenter) notifCenter.hidden = true;
    spotlight.hidden = !open;
    if (open) {
      spotInput.value = "";
      spotResults.innerHTML = `<div class="spotlight-section">Search…</div>`;
      spotHits = [];
      setTimeout(() => spotInput.focus(), 30);
    }
  }
  async function searchSpotlight(q) {
    q = (q || "").trim();
    if (!q) {
      spotResults.innerHTML = `<div class="spotlight-section">Type to search</div>`;
      return;
    }
    try {
      const data = await (await fetch("/api/search?q=" + encodeURIComponent(q) + "&limit=30")).json();
      spotHits = data.results || [];
      spotIndex = 0;
      renderSpot();
    } catch (_) {
      spotResults.innerHTML = `<div class="spotlight-section">Search failed</div>`;
    }
  }
  function renderSpot() {
    if (!spotHits.length) {
      spotResults.innerHTML = `<div class="spotlight-section">No results</div>`;
      return;
    }
    spotResults.innerHTML = spotHits
      .map(
        (h, i) => `<button type="button" class="spotlight-item${i === spotIndex ? " active" : ""}" data-idx="${i}">
      <span class="spot-icon" data-icon="${esc(h.app_id || "finder")}"></span>
      <span style="flex:1;min-width:0"><span style="display:block">${esc(h.title)}</span>
      <span style="display:block;font-size:11px;opacity:.65">${esc(h.subtitle)}</span></span></button>`
      )
      .join("");
    spotResults.querySelectorAll(".spotlight-item").forEach((item) => {
      const ic = item.querySelector(".spot-icon");
      ic.replaceWith(createIcon(ic.dataset.icon, 28));
      item.onmouseenter = () => {
        spotIndex = +item.dataset.idx;
        renderSpot();
      };
      item.onclick = () => activateSpot(+item.dataset.idx);
    });
  }
  function activateSpot(idx) {
    const h = spotHits[idx];
    if (!h) return;
    toggleSpotlight(false);
    if (h.kind === "app") openApp(h.id);
    else if (h.kind === "note") openApp("notes", { noteId: h.id });
    else if (h.kind === "reminder") openApp("reminders");
    else if (h.kind === "file") {
      const path = h.path || h.id;
      if ((h.subtitle || "").toLowerCase().includes("folder")) openApp("finder", { path });
      else openApp("textedit", { path });
    } else if (h.app_id) openApp(h.app_id);
  }
  spotInput?.addEventListener("input", () => {
    clearTimeout(spotTimer);
    spotTimer = setTimeout(() => searchSpotlight(spotInput.value), 120);
  });
  spotInput?.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (spotHits.length) {
        spotIndex = (spotIndex + 1) % spotHits.length;
        renderSpot();
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (spotHits.length) {
        spotIndex = (spotIndex - 1 + spotHits.length) % spotHits.length;
        renderSpot();
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      activateSpot(spotIndex);
    } else if (e.key === "Escape") toggleSpotlight(false);
  });
  spotlight?.addEventListener("click", (e) => {
    if (e.target.id === "spotlight") toggleSpotlight(false);
  });

  // ── Mission Control + Spaces strip ──
  const mc = document.getElementById("mission-control");
  function toggleMissionControl(show) {
    const open = show !== undefined ? show : mc.hidden;
    closeMenus();
    if (notifCenter) notifCenter.hidden = true;
    if (spotlight) spotlight.hidden = true;
    mc.hidden = !open;
    if (open) renderMissionControl();
  }

  function wallClass(wp) {
    return "wallpaper wallpaper-" + (wp || "sonoma");
  }

  function renderMissionControl() {
    const spacesEl = document.getElementById("mc-spaces");
    const grid = document.getElementById("mc-grid");
    const spaces = WM.getSpaces();
    const active = WM.getActiveSpaceId();

    spacesEl.innerHTML = "";
    spaces.forEach((sp) => {
      const card = document.createElement("div");
      card.className = "mc-space-card" + (sp.id === active ? " active" : "");
      card.dataset.spaceId = sp.id;
      card.innerHTML = `<div class="mc-space-thumb"><div class="wp ${wallClass(sp.wallpaper)}"></div></div>
        <div class="mc-space-label">${esc(sp.name)}</div>`;
      card.onclick = (e) => {
        e.stopPropagation();
        WM.switchSpace(sp.id);
        renderMissionControl();
      };
      // drop windows onto space
      card.ondragover = (e) => {
        e.preventDefault();
        card.classList.add("mc-drop-hover");
      };
      card.ondragleave = () => card.classList.remove("mc-drop-hover");
      card.ondrop = (e) => {
        e.preventDefault();
        card.classList.remove("mc-drop-hover");
        const winId = e.dataTransfer.getData("text/win-id");
        if (winId) {
          WM.moveWindowToSpace(winId, sp.id);
          renderMissionControl();
        }
      };
      // double-click remove? long-press - skip
      spacesEl.appendChild(card);
    });
    const add = document.createElement("button");
    add.className = "mc-space-add";
    add.textContent = "+";
    add.title = "Add Desktop";
    add.onclick = (e) => {
      e.stopPropagation();
      const sp = WM.addSpace();
      WM.switchSpace(sp.id);
      persistSettings();
      renderMissionControl();
    };
    spacesEl.appendChild(add);

    // windows on active space
    const wins = WM.getOpenWindows(active).filter((w) => !w.minimized && !w.fullscreen);
    if (!wins.length) {
      grid.innerHTML = `<div class="mc-empty">No windows on this Desktop — drag apps here or open new ones</div>`;
    } else {
      grid.innerHTML = "";
      wins.forEach((w) => {
        const card = document.createElement("button");
        card.className = "mc-card";
        card.draggable = true;
        card.innerHTML = `<div class="mc-thumb"><div class="mc-thumb-inner"></div></div>
          <div class="mc-label"><span class="mc-icon"></span><span>${esc(w.title || w.appId)}</span></div>`;
        card.querySelector(".mc-icon").replaceWith(createIcon(w.appId, 20));
        const inner = card.querySelector(".mc-thumb-inner");
        try {
          const clone = w.el.cloneNode(true);
          clone.classList.remove("minimized", "maximized", "fullscreen-space", "space-hidden");
          clone.style.cssText =
            "position:relative;left:0;top:0;width:560px;height:340px;transform:none;animation:none;pointer-events:none";
          clone.querySelectorAll("iframe").forEach((f) => {
            const ph = document.createElement("div");
            ph.style.cssText =
              "width:100%;height:100%;background:#e8e8ed;display:grid;place-items:center;color:#6e6e73";
            ph.textContent = "Web";
            f.replaceWith(ph);
          });
          inner.appendChild(clone);
        } catch (_) {
          inner.innerHTML = `<div style="padding:40px;color:#fff;text-align:center">${esc(w.title)}</div>`;
        }
        card.ondragstart = (e) => {
          e.dataTransfer.setData("text/win-id", w.id);
        };
        card.onclick = () => {
          toggleMissionControl(false);
          w.el.classList.remove("minimized");
          WM.focusWindow(w.el);
        };
        grid.appendChild(card);
      });
    }
  }

  document.getElementById("mc-btn")?.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleMissionControl();
  });
  mc?.addEventListener("click", (e) => {
    if (e.target.id === "mission-control" || e.target.classList.contains("mc-header") || e.target.classList.contains("mc-hint"))
      toggleMissionControl(false);
  });

  // Context menu
  const ctx = document.getElementById("context-menu");
  document.addEventListener("contextmenu", (e) => {
    if (e.target.closest(".window") || e.target.closest(".dock") || e.target.closest(".menubar")) return;
    e.preventDefault();
    ctx.hidden = false;
    ctx.style.left = e.clientX + "px";
    ctx.style.top = e.clientY + "px";
    ctx.innerHTML = `<button data-act="finder">Open Finder</button>
      <button data-act="terminal">Open Terminal</button>
      <button data-act="safari">Open Safari</button><hr/>
      <button data-act="mission">Mission Control</button>
      <button data-act="newspace">New Desktop</button>
      <button data-act="settings">System Settings</button>`;
    ctx.querySelectorAll("button").forEach((b) => {
      b.onclick = () => {
        ctx.hidden = true;
        if (b.dataset.act === "mission") toggleMissionControl(true);
        else if (b.dataset.act === "newspace") {
          const sp = WM.addSpace();
          WM.switchSpace(sp.id);
          persistSettings();
          toast("Spaces", "Created " + sp.name);
        } else if (b.dataset.act === "settings") openApp("systemsettings");
        else openApp(b.dataset.act);
      };
    });
  });

  function closeMenus() {
    if (appleDrop) {
      appleDrop.hidden = true;
      appleBtn?.classList.remove("open");
    }
    if (cc) cc.hidden = true;
    if (ctx) ctx.hidden = true;
  }

  document.addEventListener("click", (e) => {
    if (!e.target.closest(".dropdown") && !e.target.closest("#apple-btn")) {
      if (appleDrop) appleDrop.hidden = true;
      appleBtn?.classList.remove("open");
    }
    if (!e.target.closest("#control-center") && !e.target.closest("#cc-btn")) if (cc) cc.hidden = true;
    if (!e.target.closest("#context-menu")) if (ctx) ctx.hidden = true;
    if (!e.target.closest("#notif-center") && !e.target.closest("#notif-btn"))
      if (notifCenter) notifCenter.hidden = true;
  });

  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.code === "Space") {
      e.preventDefault();
      toggleSpotlight();
    }
    // Spaces: Ctrl+Left / Ctrl+Right
    if (e.ctrlKey && e.key === "ArrowLeft") {
      e.preventDefault();
      WM.switchSpaceByDelta(-1);
    }
    if (e.ctrlKey && e.key === "ArrowRight") {
      e.preventDefault();
      WM.switchSpaceByDelta(1);
    }
    if (e.ctrlKey && e.key === "ArrowUp") {
      e.preventDefault();
      toggleMissionControl(true);
    }
    if (e.ctrlKey && e.key === "ArrowDown") {
      e.preventDefault();
      toggleMissionControl(false);
    }
    if (e.key === "F3" || e.key === "F9") {
      e.preventDefault();
      toggleMissionControl();
    }
    if (e.key === "F4" || ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "l")) {
      e.preventDefault();
      toggleLaunchpad();
    }
    if (e.key === "Escape") {
      toggleLaunchpad(false);
      toggleSpotlight(false);
      toggleMissionControl(false);
      WM.exitFullscreenSpace?.();
      closeMenus();
      if (notifCenter) notifCenter.hidden = true;
      const about = document.getElementById("about-modal");
      if (about) about.hidden = true;
    }
  });

  buildDock();
  buildDesktopIcons();
  refreshNotifications();
  setInterval(refreshNotifications, 15000);

  setTimeout(
    () =>
      toast(
        "Welcome, " + (M.username || "User"),
        "Ctrl+←/→ Spaces · F3 Mission Control · drag edges to snap windows"
      ),
    500
  );
  setTimeout(() => openApp("finder", { path: "~/Desktop" }), 800);

  window.MaxcosDesktop = {
    toast,
    toggleLaunchpad,
    toggleSpotlight,
    toggleNotifCenter,
    toggleMissionControl,
    refreshNotifications,
    openApp,
    renderMissionControl,
    persistSettings,
    onSpaceChanged: () => {
      // could refresh desktop icons per space later
    },
  };
})();
