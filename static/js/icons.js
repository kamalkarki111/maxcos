
(function(g){
  const C={finder:"#0a84ff",safari:"#0a84ff",messages:"#30d158",mail:"#007aff",maps:"#34c759",photos:"conic-gradient(from 180deg,#ff2d55,#ff9f0a,#ffd60a,#30d158,#64d2ff,#bf5af2)",
    facetime:"#30d158",calendar:"#fff",notes:"#ffe08a",reminders:"#f2f2f7",music:"#ff2d55",tv:"#000",podcasts:"#9b30ff",appstore:"#0a84ff",settings:"#636366",
    calculator:"#1c1c1e",terminal:"#1c1c1e",textedit:"#e8e8ed",preview:"#0a84ff",clock:"#1c1c1e",weather:"#0a84ff",contacts:"#636366",books:"#ff9500",launchpad:"#1c1c1e",trash:"#636366"};
  const G={finder:"📁",safari:"🧭",messages:"💬",mail:"✉️",maps:"🗺️",photos:"🖼️",facetime:"📹",calendar:"📅",notes:"📝",reminders:"✓",music:"🎵",tv:"tv",
    podcasts:"🎙️",appstore:"A",settings:"⚙️",calculator:"🧮",terminal:">_",textedit:"Aa",preview:"👁",clock:"🕐",weather:"☀️",contacts:"👤",books:"📚",launchpad:"⬚",trash:"🗑"};
  function createIcon(id,size){
    const el=document.createElement("div");
    el.className="app-icon";
    el.style.width=size?size+"px":"";
    el.style.height=size?size+"px":"";
    el.style.background=C[id]||"#8e8e93";
    el.style.display="grid";
    el.style.placeItems="center";
    el.style.color=(id==="notes"||id==="textedit"||id==="calendar"||id==="reminders")?"#1d1d1f":"#fff";
    el.style.fontSize=size?Math.round(size*0.4)+"px":"18px";
    el.style.fontWeight="700";
    el.textContent=G[id]||"•";
    return el;
  }
  g.MaxcosIcons={createIcon};
})(window);
