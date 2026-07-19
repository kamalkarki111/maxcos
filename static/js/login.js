
(function () {
  const users = JSON.parse(document.getElementById("users-data").textContent || "[]");
  const colors = ["#0A84FF", "#30D158", "#FF9F0A", "#FF375F", "#BF5AF2", "#64D2FF", "#8E8E93"];
  let selected = null;
  let nuColor = colors[0];

  const picker = document.getElementById("user-picker");
  const form = document.getElementById("login-form");
  const userId = document.getElementById("user_id");
  const sel = document.getElementById("selected-user");
  const selAvatar = document.getElementById("sel-avatar");
  const selName = document.getElementById("sel-name");
  const pwWrap = document.getElementById("password-wrap");
  const enterBtn = document.getElementById("login-enter");
  const hint = document.getElementById("hint");
  const password = document.getElementById("password");

  function renderUsers() {
    picker.innerHTML = "";
    if (!users.length) {
      picker.innerHTML = `<p style="color:rgba(255,255,255,.7);text-align:center">No users yet. <a href="/setup" style="color:#fff">Set up this Mac</a></p>`;
      return;
    }
    users.forEach((u) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "user-chip" + (selected && selected.id === u.id ? " active" : "");
      b.innerHTML = `<div class="user-avatar" style="background:${u.color}">${escapeHtml(u.avatar)}</div><span>${escapeHtml(u.name)}</span>`;
      b.onclick = () => select(u);
      picker.appendChild(b);
    });
  }

  function select(u) {
    selected = u;
    userId.value = u.id;
    sel.hidden = false;
    selAvatar.style.background = u.color;
    selAvatar.textContent = u.avatar;
    selName.textContent = u.name;
    renderUsers();
    if (u.has_password) {
      pwWrap.hidden = false;
      enterBtn.hidden = true;
      if (u.password_hint) {
        hint.hidden = false;
        hint.textContent = "Hint: " + u.password_hint;
      } else {
        hint.hidden = true;
      }
      password.value = "";
      password.focus();
    } else {
      pwWrap.hidden = true;
      enterBtn.hidden = true;
      hint.hidden = true;
      form.submit();
    }
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  renderUsers();

  // Add user sheet
  const sheet = document.getElementById("add-sheet");
  const nuPreview = document.getElementById("nu-preview");
  const nuSw = document.getElementById("nu-swatches");
  const nuName = document.getElementById("nu-name");
  document.getElementById("btn-add-user").onclick = () => {
    sheet.hidden = false;
    nuName.focus();
  };
  document.getElementById("nu-cancel").onclick = () => {
    sheet.hidden = true;
  };
  sheet.addEventListener("click", (e) => {
    if (e.target === sheet) sheet.hidden = true;
  });

  function initials(name) {
    const p = (name || "").trim().split(/\s+/).filter(Boolean);
    if (!p.length) return "?";
    if (p.length === 1) return p[0].slice(0, 1).toUpperCase();
    return (p[0][0] + p[p.length - 1][0]).toUpperCase();
  }
  function updateNu() {
    nuPreview.textContent = initials(nuName.value);
    nuPreview.style.background = nuColor;
  }
  colors.forEach((c, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.style.background = c;
    if (i === 0) b.classList.add("active");
    b.onclick = () => {
      nuColor = c;
      nuSw.querySelectorAll("button").forEach((x) => x.classList.remove("active"));
      b.classList.add("active");
      updateNu();
    };
    nuSw.appendChild(b);
  });
  nuName.addEventListener("input", updateNu);
  updateNu();

  document.getElementById("nu-create").onclick = async () => {
    const err = document.getElementById("nu-error");
    err.hidden = true;
    const name = nuName.value.trim();
    const pass = document.getElementById("nu-pass").value;
    const pass2 = document.getElementById("nu-pass2").value;
    if (!name) {
      err.textContent = "Enter a name.";
      err.hidden = false;
      return;
    }
    if (pass !== pass2) {
      err.textContent = "Passwords do not match.";
      err.hidden = false;
      return;
    }
    if (!pass || pass.length < 8) {
      err.textContent = "Password must be at least 8 characters.";
      err.hidden = false;
      return;
    }
    if (!/\d/.test(pass)) {
      err.textContent = "Password must contain at least one number.";
      err.hidden = false;
      return;
    }
    const r = await fetch("/api/users", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name,
        password: pass,
        password_hint: document.getElementById("nu-hint").value,
        avatar: initials(name),
        color: nuColor,
        sign_in: false,
      }),
    });
    if (!r.ok) {
      err.textContent = await r.text();
      err.hidden = false;
      return;
    }
    const u = await r.json();
    users.push({
      id: u.id,
      name: u.name,
      avatar: u.avatar,
      color: u.color,
      has_password: true,
      password_hint: document.getElementById("nu-hint").value || "",
    });
    sheet.hidden = true;
    nuName.value = "";
    document.getElementById("nu-pass").value = "";
    document.getElementById("nu-pass2").value = "";
    renderUsers();
    select(users[users.length - 1]);
  };

  // clock
  async function tick() {
    try {
      const d = await (await fetch("/api/time")).json();
      document.getElementById("login-time").textContent = d.time_short;
      document.getElementById("login-date").textContent = (d.date_full || "").replace(/, \d{4}$/, "");
    } catch (_) {}
  }
  setInterval(tick, 30000);
})();
