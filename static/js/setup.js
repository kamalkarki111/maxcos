
(function () {
  const colors = ["#0A84FF", "#30D158", "#FF9F0A", "#FF375F", "#BF5AF2", "#64D2FF", "#8E8E93"];
  let color = colors[0];
  const sw = document.getElementById("swatches");
  const preview = document.getElementById("avatar-preview");
  const fullName = document.getElementById("full_name");
  const accountName = document.getElementById("account_name");
  const colorInput = document.getElementById("color");
  const avatarInput = document.getElementById("avatar");

  function initials(name) {
    const p = (name || "").trim().split(/\s+/).filter(Boolean);
    if (!p.length) return "?";
    if (p.length === 1) return p[0].slice(0, 1).toUpperCase();
    return (p[0][0] + p[p.length - 1][0]).toUpperCase();
  }
  function slug(name) {
    return (name || "")
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "")
      .slice(0, 20);
  }
  function updatePreview() {
    const n = fullName.value;
    preview.textContent = initials(n);
    preview.style.background = color;
    avatarInput.value = initials(n);
    colorInput.value = color;
    if (!accountName.dataset.touched) accountName.value = slug(n);
  }
  colors.forEach((c, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.style.background = c;
    if (i === 0) b.classList.add("active");
    b.onclick = () => {
      color = c;
      sw.querySelectorAll("button").forEach((x) => x.classList.remove("active"));
      b.classList.add("active");
      updatePreview();
    };
    sw.appendChild(b);
  });
  fullName.addEventListener("input", updatePreview);
  accountName.addEventListener("input", () => {
    accountName.dataset.touched = "1";
  });
  updatePreview();

  const panels = [
    document.getElementById("step-hello"),
    document.getElementById("step-account"),
    document.getElementById("step-done"),
  ];
  const dots = document.querySelectorAll(".setup-progress .dot");
  function go(step) {
    panels.forEach((p, i) => p.classList.toggle("active", i === step));
    dots.forEach((d, i) => d.classList.toggle("active", i === step));
  }
  document.getElementById("btn-continue-hello").onclick = () => {
    go(1);
    fullName.focus();
  };
  document.getElementById("btn-back-account").onclick = () => go(0);
  document.getElementById("setup-form").addEventListener("submit", (e) => {
    const p1 = document.getElementById("password").value;
    const p2 = document.getElementById("password_confirm").value;
    if (p1 !== p2) {
      e.preventDefault();
      alert("Passwords do not match.");
      return;
    }
    if (!fullName.value.trim()) {
      e.preventDefault();
      return;
    }
    go(2);
  });
})();
