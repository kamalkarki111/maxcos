
/**
 * Window Manager — multi-space, snap, fullscreen space
 */
(function (g) {
  let z = 100;
  const windows = new Map();
  let activeSpaceId = "space-1";
  let spaces = [{ id: "space-1", name: "Desktop 1", wallpaper: "sonoma" }];
  let fullscreenWinId = null;

  function nextZ() { return ++z; }

  function menubarH() {
    return parseInt(getComputedStyle(document.documentElement).getPropertyValue("--menubar-h")) || 28;
  }

  function setSpaces(list, activeId) {
    spaces = list && list.length ? list : spaces;
    if (activeId) activeSpaceId = activeId;
    else if (!spaces.find((s) => s.id === activeSpaceId)) activeSpaceId = spaces[0].id;
    applySpaceVisibility();
    applyWallpaper();
  }

  function getSpaces() { return spaces.slice(); }
  function getActiveSpaceId() { return activeSpaceId; }

  function applyWallpaper() {
    const sp = spaces.find((s) => s.id === activeSpaceId);
    const wp = sp?.wallpaper || "sonoma";
    const el = document.getElementById("wallpaper");
    if (!el) return;
    el.className = "wallpaper wallpaper-" + wp;
  }

  function applySpaceVisibility() {
    for (const [, w] of windows) {
      if (!w.el.isConnected) continue;
      const onSpace = w.spaceId === activeSpaceId;
      const isFs = w.el.dataset.winId === fullscreenWinId;
      if (isFs) {
        w.el.classList.remove("space-hidden");
        continue;
      }
      w.el.classList.toggle("space-hidden", !onSpace);
    }
  }

  function switchSpace(spaceId) {
    if (!spaces.find((s) => s.id === spaceId)) return;
    // exit fullscreen when switching
    if (fullscreenWinId) exitFullscreenSpace();
    activeSpaceId = spaceId;
    applySpaceVisibility();
    applyWallpaper();
    g.MaxcosDesktop?.onSpaceChanged?.(spaceId);
    persistSettingsSoon();
  }

  function switchSpaceByDelta(delta) {
    const idx = spaces.findIndex((s) => s.id === activeSpaceId);
    if (idx < 0) return;
    const next = (idx + delta + spaces.length) % spaces.length;
    switchSpace(spaces[next].id);
    showSpaceIndicator(spaces[next].name);
  }

  function showSpaceIndicator(name) {
    const el = document.getElementById("space-indicator");
    if (!el) return;
    el.hidden = false;
    el.textContent = name;
    clearTimeout(showSpaceIndicator._t);
    showSpaceIndicator._t = setTimeout(() => { el.hidden = true; }, 900);
  }

  let persistTimer = null;
  function persistSettingsSoon() {
    clearTimeout(persistTimer);
    persistTimer = setTimeout(() => g.MaxcosDesktop?.persistSettings?.(), 200);
  }

  function focusWindow(winEl) {
    document.querySelectorAll(".window.focused").forEach((w) => w.classList.remove("focused"));
    winEl.classList.add("focused");
    winEl.style.zIndex = nextZ();
    const nameEl = document.getElementById("active-app-name");
    if (nameEl) nameEl.textContent = winEl.dataset.title || "Finder";
    updateDockDots();
  }

  function updateDockDots() {
    const open = new Set();
    document.querySelectorAll(".window").forEach((w) => open.add(w.dataset.app));
    document.querySelectorAll(".dock-item").forEach((d) =>
      d.classList.toggle("open", open.has(d.dataset.app))
    );
  }

  function snapRect(zone) {
    const top = menubarH();
    const w = window.innerWidth;
    const h = window.innerHeight - top;
    const hw = Math.floor(w / 2);
    const hh = Math.floor(h / 2);
    switch (zone) {
      case "left": return { left: 0, top, width: hw, height: h };
      case "right": return { left: hw, top, width: w - hw, height: h };
      case "top-left": return { left: 0, top, width: hw, height: hh };
      case "top-right": return { left: hw, top, width: w - hw, height: hh };
      case "bottom-left": return { left: 0, top: top + hh, width: hw, height: h - hh };
      case "bottom-right": return { left: hw, top: top + hh, width: w - hw, height: h - hh };
      case "full": return { left: 0, top, width: w, height: h };
      default: return null;
    }
  }

  function detectSnapZone(x, y) {
    const edge = 18;
    const w = window.innerWidth;
    const h = window.innerHeight;
    const left = x <= edge;
    const right = x >= w - edge;
    const top = y <= menubarH() + edge;
    const bottom = y >= h - edge;
    if (left && top) return "top-left";
    if (right && top) return "top-right";
    if (left && bottom) return "bottom-left";
    if (right && bottom) return "bottom-right";
    if (left) return "left";
    if (right) return "right";
    if (top && x > w * 0.25 && x < w * 0.75) return "full";
    return null;
  }

  function showSnapPreview(zone) {
    const prev = document.getElementById("snap-preview");
    if (!prev) return;
    const r = snapRect(zone);
    if (!r) { prev.hidden = true; return; }
    prev.hidden = false;
    Object.assign(prev.style, {
      left: r.left + "px",
      top: r.top + "px",
      width: r.width + "px",
      height: r.height + "px",
    });
  }

  function hideSnapPreview() {
    const prev = document.getElementById("snap-preview");
    if (prev) prev.hidden = true;
  }

  function applySnap(win, zone) {
    const r = snapRect(zone);
    if (!r) return;
    win.classList.remove("maximized", "fullscreen-space");
    win.classList.add("snapped");
    Object.assign(win.style, {
      left: r.left + "px",
      top: r.top + "px",
      width: r.width + "px",
      height: r.height + "px",
    });
    win.dataset.snap = zone;
    setTimeout(() => win.classList.remove("snapped"), 200);
  }

  function enterFullscreenSpace(win) {
    // create dedicated fullscreen pseudo-space feel: hide other windows
    fullscreenWinId = win.dataset.winId;
    win._preFs = {
      left: win.style.left,
      top: win.style.top,
      width: win.style.width,
      height: win.style.height,
      spaceId: windows.get(win.dataset.winId)?.spaceId,
    };
    win.classList.remove("maximized", "snapped");
    win.classList.add("fullscreen-space");
    Object.assign(win.style, {
      left: "0px",
      top: menubarH() + "px",
      width: "100vw",
      height: `calc(100vh - ${menubarH()}px)`,
    });
    // hide others
    for (const [, w] of windows) {
      if (w.el !== win) w.el.classList.add("space-hidden");
    }
    focusWindow(win);
    const nameEl = document.getElementById("active-app-name");
    if (nameEl) nameEl.textContent = (win.dataset.title || "App") + " — Full Screen";
  }

  function exitFullscreenSpace() {
    if (!fullscreenWinId) return;
    const w = windows.get(fullscreenWinId);
    fullscreenWinId = null;
    if (w && w.el.isConnected) {
      w.el.classList.remove("fullscreen-space");
      if (w.el._preFs) {
        Object.assign(w.el.style, {
          left: w.el._preFs.left,
          top: w.el._preFs.top,
          width: w.el._preFs.width,
          height: w.el._preFs.height,
        });
      }
    }
    applySpaceVisibility();
  }

  function createWindow(opts) {
    const {
      id, appId, title, width = 700, height = 480,
      resizable = true, dark = false, content, forceNew = false, spaceId,
    } = opts;

    if (!forceNew) {
      for (const [, w] of windows) {
        if (w.appId === appId && w.el.isConnected && w.spaceId === activeSpaceId) {
          w.el.classList.remove("minimized", "space-hidden");
          if (fullscreenWinId && fullscreenWinId !== w.el.dataset.winId) exitFullscreenSpace();
          focusWindow(w.el);
          return w.el;
        }
      }
    }

    if (fullscreenWinId) exitFullscreenSpace();

    const layer = document.getElementById("windows-layer");
    const win = document.createElement("div");
    win.className = "window" + (dark ? " window-dark" : "");
    win.dataset.app = appId;
    win.dataset.title = title;
    win.dataset.winId = id;
    const sid = spaceId || activeSpaceId;
    const left = Math.max(40, (innerWidth - width) / 2 + (windows.size % 5) * 24);
    const top = Math.max(40, (innerHeight - height) / 2 - 40 + (windows.size % 5) * 24);
    Object.assign(win.style, {
      width: width + "px",
      height: height + "px",
      left: left + "px",
      top: top + "px",
      zIndex: nextZ(),
    });
    win.innerHTML = `<div class="titlebar"><div class="traffic-lights">
      <button class="tl tl-close" data-act="close" title="Close">×</button>
      <button class="tl tl-min" data-act="min" title="Minimize">−</button>
      <button class="tl tl-max" data-act="max" title="Full Screen">+</button>
    </div><div class="titlebar-title"></div></div><div class="window-body"></div>${
      resizable ? '<div class="resize-handle"></div>' : ""
    }`;
    win.querySelector(".titlebar-title").textContent = title;
    const body = win.querySelector(".window-body");
    if (typeof content === "string") body.innerHTML = content;
    else if (content instanceof Node) body.appendChild(content);

    win.querySelector('[data-act="close"]').onclick = (e) => {
      e.stopPropagation();
      closeWindow(win);
    };
    win.querySelector('[data-act="min"]').onclick = (e) => {
      e.stopPropagation();
      win.classList.add("minimized");
      updateDockDots();
    };
    win.querySelector('[data-act="max"]').onclick = (e) => {
      e.stopPropagation();
      if (e.altKey) {
        // Alt+green: classic tile maximize
        if (win.classList.contains("fullscreen-space")) exitFullscreenSpace();
        if (win.classList.contains("maximized")) {
          win.classList.remove("maximized");
          if (win._restore) Object.assign(win.style, win._restore);
        } else {
          win._restore = {
            left: win.style.left, top: win.style.top,
            width: win.style.width, height: win.style.height,
          };
          win.classList.add("maximized");
        }
      } else {
        // Green: true fullscreen space
        if (win.classList.contains("fullscreen-space") || fullscreenWinId === id) {
          exitFullscreenSpace();
        } else {
          enterFullscreenSpace(win);
        }
      }
      focusWindow(win);
    };

    win.addEventListener("mousedown", () => focusWindow(win));

    // Drag + snap
    const titlebar = win.querySelector(".titlebar");
    let dragging = false, ox = 0, oy = 0;
    titlebar.addEventListener("mousedown", (e) => {
      if (e.target.closest(".tl") || win.classList.contains("fullscreen-space")) return;
      if (win.classList.contains("maximized")) {
        win.classList.remove("maximized");
        if (win._restore) Object.assign(win.style, win._restore);
      }
      dragging = true;
      ox = e.clientX - win.offsetLeft;
      oy = e.clientY - win.offsetTop;
      focusWindow(win);
      e.preventDefault();
    });
    window.addEventListener("mousemove", (e) => {
      if (!dragging) return;
      win.style.left = Math.max(0, e.clientX - ox) + "px";
      win.style.top = Math.max(menubarH(), e.clientY - oy) + "px";
      const zone = detectSnapZone(e.clientX, e.clientY);
      if (zone) showSnapPreview(zone);
      else hideSnapPreview();
    });
    window.addEventListener("mouseup", (e) => {
      if (!dragging) return;
      dragging = false;
      const zone = detectSnapZone(e.clientX, e.clientY);
      hideSnapPreview();
      if (zone) applySnap(win, zone);
    });

    // Resize
    const handle = win.querySelector(".resize-handle");
    if (handle) {
      let resizing = false, sx, sy, sw, sh;
      handle.addEventListener("mousedown", (e) => {
        if (win.classList.contains("maximized") || win.classList.contains("fullscreen-space")) return;
        resizing = true;
        sx = e.clientX; sy = e.clientY;
        sw = win.offsetWidth; sh = win.offsetHeight;
        e.preventDefault(); e.stopPropagation();
      });
      window.addEventListener("mousemove", (e) => {
        if (!resizing) return;
        win.style.width = Math.max(280, sw + (e.clientX - sx)) + "px";
        win.style.height = Math.max(180, sh + (e.clientY - sy)) + "px";
      });
      window.addEventListener("mouseup", () => { resizing = false; });
    }

    layer.appendChild(win);
    windows.set(id, { el: win, appId, meta: opts, spaceId: sid });
    applySpaceVisibility();
    focusWindow(win);
    return win;
  }

  function closeWindow(win) {
    const id = win.dataset.winId;
    if (fullscreenWinId === id) exitFullscreenSpace();
    win.remove();
    windows.delete(id);
    updateDockDots();
    const rem = [...document.querySelectorAll(".window:not(.minimized):not(.space-hidden)")];
    if (rem.length) {
      rem.sort((a, b) => (+b.style.zIndex || 0) - (+a.style.zIndex || 0));
      focusWindow(rem[0]);
    } else {
      const n = document.getElementById("active-app-name");
      if (n) n.textContent = "Finder";
    }
  }

  function findWindowByApp(appId) {
    for (const [, w] of windows) {
      if (w.appId === appId && w.el.isConnected && w.spaceId === activeSpaceId) return w.el;
    }
    // fallback any space
    for (const [, w] of windows) {
      if (w.appId === appId && w.el.isConnected) return w.el;
    }
    return null;
  }

  function getOpenWindows(spaceId) {
    const sid = spaceId || activeSpaceId;
    return [...windows.values()]
      .filter((w) => w.el.isConnected && (spaceId === "*" || w.spaceId === sid))
      .map((w) => ({
        id: w.el.dataset.winId,
        appId: w.appId,
        title: w.el.dataset.title,
        el: w.el,
        spaceId: w.spaceId,
        minimized: w.el.classList.contains("minimized"),
        fullscreen: w.el.classList.contains("fullscreen-space"),
      }));
  }

  function moveWindowToSpace(winId, spaceId) {
    const w = windows.get(winId);
    if (!w || !spaces.find((s) => s.id === spaceId)) return;
    w.spaceId = spaceId;
    if (fullscreenWinId === winId) exitFullscreenSpace();
    applySpaceVisibility();
  }

  function addSpace(wallpaper) {
    const n = spaces.length + 1;
    const walls = ["sonoma", "sequoia", "midnight", "dawn"];
    const sp = {
      id: "space-" + Date.now(),
      name: "Desktop " + n,
      wallpaper: wallpaper || walls[spaces.length % walls.length],
    };
    spaces.push(sp);
    persistSettingsSoon();
    return sp;
  }

  function removeSpace(spaceId) {
    if (spaces.length <= 1) return false;
    const idx = spaces.findIndex((s) => s.id === spaceId);
    if (idx < 0) return false;
    const fallback = spaces[idx === 0 ? 1 : 0].id;
    for (const [, w] of windows) {
      if (w.spaceId === spaceId) w.spaceId = fallback;
    }
    spaces.splice(idx, 1);
    if (activeSpaceId === spaceId) switchSpace(fallback);
    else applySpaceVisibility();
    persistSettingsSoon();
    return true;
  }

  g.MaxcosWM = {
    createWindow,
    closeWindow,
    focusWindow,
    findWindowByApp,
    getOpenWindows,
    windows,
    setSpaces,
    getSpaces,
    getActiveSpaceId,
    switchSpace,
    switchSpaceByDelta,
    moveWindowToSpace,
    addSpace,
    removeSpace,
    applyWallpaper,
    applySpaceVisibility,
    exitFullscreenSpace,
    enterFullscreenSpace,
  };
})(window);
