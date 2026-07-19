
(function(g){
  const {createWindow}=g.MaxcosWM;
  const {createIcon}=g.MaxcosIcons;
  function uid(){return "w-"+Math.random().toString(36).slice(2,10)}
  function appMeta(id){return (g.MAXCOS.allApps||[]).find(a=>a.id===id)||{id,name:id,width:700,height:480,resizable:true}}
  function esc(s){return String(s??"").replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;")}
  async function pushNotification(title,body,app,appId){
    try{
      await fetch("/api/notifications",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({title,body,app:app||"Maxcos",app_id:appId||"systemsettings"})});
      g.MaxcosDesktop?.refreshNotifications?.();
    }catch(_){}
  }

  async function openApp(appId,opts){
    opts=opts||{};
    if(appId==="launchpad"){g.MaxcosDesktop.toggleLaunchpad();return}
    const meta=appMeta(appId);
    const builders={
      finder:buildFinder,safari:buildSafari,messages:buildMessages,mail:buildMail,maps:buildMaps,
      photos:buildPhotos,facetime:buildFaceTime,calendar:buildCalendar,notes:buildNotes,reminders:buildReminders,
      music:buildMusic,tv:buildTV,podcasts:buildPodcasts,appstore:buildAppStore,systemsettings:buildSettings,
      calculator:buildCalculator,terminal:buildTerminal,textedit:buildTextEdit,preview:buildPreview,
      clock:buildClock,weather:buildWeather,contacts:buildContacts,books:buildBooks,trash:buildTrash
    };
    const build=builders[appId];
    if(!build){
      createWindow({id:uid(),appId,title:meta.name,width:meta.width||600,height:meta.height||400,content:`<div style="padding:40px;text-align:center;color:#6e6e73">${meta.name}</div>`});
      return;
    }
    const content=await build(opts);
    const dark=["terminal","calculator","clock","facetime","tv"].includes(appId);
    let title=meta.name;
    if(appId==="textedit"&&opts.path) title=opts.path.split("/").pop()||"TextEdit";
    if(appId==="finder"&&opts.path) title="Finder — "+(opts.path.replace(/^~\//,"")||"Home");
    createWindow({id:uid(),appId,title,width:meta.width||700,height:meta.height||480,resizable:meta.resizable!==false,dark,content,forceNew:appId==="textedit"||appId==="finder"||appId==="safari"||appId==="terminal"});
  }

  // ── Safari (real proxy) ──
  async function buildSafari(){
    const root=document.createElement("div");
    root.className="safari-body";
    let state={tabs:[],bookmarks:[],history:[]};
    try{ state=await (await fetch("/api/safari")).json(); }catch(_){}
    if(!state.tabs||!state.tabs.length){
      state.tabs=[{id:crypto.randomUUID?.()||uid(),title:"Start Page",url:"about:start",active:true}];
    }
    let activeId=state.tabs.find(t=>t.active)?.id||state.tabs[0].id;

    function activeTab(){return state.tabs.find(t=>t.id===activeId)||state.tabs[0]}

    async function persist(){
      state.tabs.forEach(t=>t.active=(t.id===activeId));
      await fetch("/api/safari",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({tabs:state.tabs,bookmarks:state.bookmarks})});
    }

    function render(){
      const tab=activeTab();
      root.innerHTML=`
        <div class="safari-tabs" id="s-tabs"></div>
        <div class="safari-nav">
          <button class="tb-btn" id="s-back">◀</button>
          <button class="tb-btn" id="s-fwd">▶</button>
          <button class="tb-btn" id="s-reload">↻</button>
          <input class="safari-url" id="s-url" value="${esc(tab.url==="about:start"?"":tab.url)}" placeholder="Search or enter website"/>
          <button class="tb-btn" id="s-home">⌂</button>
          <button class="tb-btn" id="s-star" title="Bookmark">☆</button>
        </div>
        <iframe class="safari-frame" id="s-frame" title="Safari" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"></iframe>`;
      const tabsEl=root.querySelector("#s-tabs");
      state.tabs.forEach(t=>{
        const b=document.createElement("button");
        b.className="safari-tab"+(t.id===activeId?" active":"");
        b.innerHTML=`<span>${esc(t.title||"Tab")}</span><span class="tab-x" data-close="${t.id}">×</span>`;
        b.onclick=e=>{
          if(e.target.dataset.close){
            e.stopPropagation();
            closeTab(t.id);
            return;
          }
          activeId=t.id; render(); loadFrame();
        };
        tabsEl.appendChild(b);
      });
      const add=document.createElement("button");
      add.className="safari-tab-add"; add.textContent="+";
      add.onclick=()=>{
        const id=crypto.randomUUID?.()||uid();
        state.tabs.push({id,title:"Start Page",url:"about:start",active:false});
        activeId=id; persist(); render(); loadFrame();
      };
      tabsEl.appendChild(add);

      root.querySelector("#s-url").onkeydown=e=>{ if(e.key==="Enter") navigate(e.target.value) };
      root.querySelector("#s-home").onclick=()=>navigate("about:start");
      root.querySelector("#s-reload").onclick=()=>loadFrame(true);
      root.querySelector("#s-back").onclick=()=>historyNav(-1);
      root.querySelector("#s-fwd").onclick=()=>historyNav(1);
      root.querySelector("#s-star").onclick=async()=>{
        const t=activeTab();
        if(t.url==="about:start") return;
        state.bookmarks=state.bookmarks||[];
        if(!state.bookmarks.some(b=>b.url===t.url)){
          state.bookmarks.push({id:uid(),title:t.title||t.url,url:t.url});
          await persist();
          g.MaxcosDesktop?.toast?.("Safari","Bookmark saved");
        }
      };
      // track iframe navigations within proxy
      const frame=root.querySelector("#s-frame");
      frame.addEventListener("load",()=>{
        try{
          const loc=frame.contentWindow.location.href;
          if(loc.includes("/proxy")){
            const u=new URL(loc, location.origin);
            const proxied=u.searchParams.get("url");
            if(proxied){
              const tab=activeTab();
              tab.url=proxied;
              // title from document
              try{ tab.title=frame.contentDocument?.title||proxied; }catch(_){ tab.title=proxied; }
              root.querySelector("#s-url").value=proxied;
              const win=root.closest(".window");
              if(win){ win.dataset.title=tab.title; const tt=win.querySelector(".titlebar-title"); if(tt) tt.textContent=tab.title; }
              fetch("/api/safari/navigate",{method:"POST",headers:{"Content-Type":"application/json"},
                body:JSON.stringify({url:proxied,title:tab.title,tab_id:tab.id})});
              persist();
            }
          }
        }catch(_){}
      });
    }

    function closeTab(id){
      if(state.tabs.length<=1){
        state.tabs=[{id:uid(),title:"Start Page",url:"about:start",active:true}];
        activeId=state.tabs[0].id;
      }else{
        state.tabs=state.tabs.filter(t=>t.id!==id);
        if(activeId===id) activeId=state.tabs[0].id;
      }
      persist(); render(); loadFrame();
    }

    function proxySrc(url){
      if(!url||url==="about:start"||url==="about:blank") return "/proxy?url=about:start";
      return "/proxy?url="+encodeURIComponent(url);
    }

    function loadFrame(){
      const tab=activeTab();
      const frame=root.querySelector("#s-frame");
      if(frame) frame.src=proxySrc(tab.url);
      const urlInput=root.querySelector("#s-url");
      if(urlInput) urlInput.value=tab.url==="about:start"?"":tab.url;
    }

    async function navigate(raw){
      let url=(raw||"").trim();
      if(!url){ url="about:start"; }
      else if(!/^https?:\/\//i.test(url) && url!=="about:start"){
        if(url.includes(".") && !url.includes(" ")) url="https://"+url;
        else url="https://duckduckgo.com/?q="+encodeURIComponent(url);
      }
      const tab=activeTab();
      tab.url=url;
      tab.title=url==="about:start"?"Start Page":url;
      // stack for back/forward
      tab._hist=tab._hist||[];
      tab._histIdx=tab._histIdx??-1;
      tab._hist=tab._hist.slice(0, tab._histIdx+1);
      tab._hist.push(url);
      tab._histIdx=tab._hist.length-1;
      await persist();
      render();
      loadFrame();
      if(url!=="about:start"){
        fetch("/api/safari/navigate",{method:"POST",headers:{"Content-Type":"application/json"},
          body:JSON.stringify({url,title:tab.title,tab_id:tab.id})});
      }
    }

    function historyNav(dir){
      const tab=activeTab();
      tab._hist=tab._hist||[tab.url];
      tab._histIdx=tab._histIdx??(tab._hist.length-1);
      const next=tab._histIdx+dir;
      if(next<0||next>=tab._hist.length) return;
      tab._histIdx=next;
      tab.url=tab._hist[next];
      render(); loadFrame();
    }

    // init hist
    state.tabs.forEach(t=>{ t._hist=[t.url]; t._histIdx=0; });
    render();
    loadFrame();
    return root;
  }

  // ── Terminal (real sandbox) ──
  function buildTerminal(){
    const root=document.createElement("div");
    root.className="term-body";
    let cwd="~";
    let history=[];
    let histIdx=-1;
    root.innerHTML=`<div id="term-out"></div>
      <div class="term-input-line">
        <span class="term-prompt" id="term-prompt"></span>
        <input class="term-input" id="term-in" autocomplete="off" spellcheck="false"/>
      </div>`;
    const out=root.querySelector("#term-out");
    const input=root.querySelector("#term-in");
    const prompt=root.querySelector("#term-prompt");
    function setPrompt(){
      const short = (cwd === "~" || cwd === "/Users/maxcos") ? "~" : cwd.replace(/^\/Users\/maxcos/, "~");
      prompt.textContent = "maxcos@Maxcos-MacBook-Pro " + short + " %";
      prompt.style.color = "#6a9955";
    }
    function print(htmlOrText, isCmd){
      const line=document.createElement("div");
      line.className="term-line";
      if(isCmd){
        line.innerHTML=`<span class="term-prompt">${esc(prompt.textContent)}</span> ${esc(htmlOrText)}`;
      }else{
        // basic ANSI → HTML
        line.innerHTML=ansiToHtml(htmlOrText);
      }
      out.appendChild(line);
      root.scrollTop=root.scrollHeight;
    }
    function ansiToHtml(s){
      // convert common SGR sequences
      let h=esc(s);
      h=h.replace(/\x1b\[0m/g,"</span>");
      h=h.replace(/\x1b\[1;36m/g,'<span style="color:#4ec9b0;font-weight:600">');
      h=h.replace(/\x1b\[1m/g,'<span style="font-weight:600">');
      h=h.replace(/\x1b\[36m/g,'<span style="color:#4ec9b0">');
      h=h.replace(/\x1b\[33m/g,'<span style="color:#dcdcaa">');
      h=h.replace(/\x1b\[31m/g,'<span style="color:#f44747">');
      h=h.replace(/\x1b\[[0-9;]*m/g,"");
      return h;
    }
    setPrompt();
    print("Last login: "+new Date().toString());
    print("\x1b[1;36mMaxcos Terminal\x1b[0m — real sandboxed shell. Type \x1b[33mhelp\x1b[0m.");
    input.addEventListener("keydown",async e=>{
      if(e.key==="ArrowUp"){
        e.preventDefault();
        if(!history.length) return;
        histIdx=histIdx<0?history.length-1:Math.max(0,histIdx-1);
        input.value=history[histIdx]; return;
      }
      if(e.key==="ArrowDown"){
        e.preventDefault();
        if(histIdx<0) return;
        histIdx++;
        if(histIdx>=history.length){histIdx=-1;input.value=""}
        else input.value=history[histIdx];
        return;
      }
      if(e.key!=="Enter") return;
      const cmd=input.value;
      input.value=""; histIdx=-1;
      if(cmd.trim()) history.push(cmd);
      print(cmd,true);
      try{
        const r=await fetch("/api/terminal",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({cmd,cwd})});
        const data=await r.json();
        cwd=data.cwd||cwd;
        setPrompt();
        if(data.output==="__CLEAR__"){ out.innerHTML=""; return; }
        if(data.output==="__EXIT__"){
          print("logout");
          const win=root.closest(".window");
          if(win) g.MaxcosWM.closeWindow(win);
          return;
        }
        if(data.output) print(data.output);
        if(cmd.trim().startsWith("open ")){
          const app=cmd.trim().slice(5).trim().toLowerCase().replace(/\s+/g,"");
          const map={finder:"finder",safari:"safari",notes:"notes",terminal:"terminal",calculator:"calculator"};
          if(map[app]) openApp(map[app]);
        }
      }catch(err){
        print("\x1b[31merror: "+err+"\x1b[0m");
      }
    });
    setTimeout(()=>input.focus(),100);
    root.onclick=()=>input.focus();
    return root;
  }

  // Finder
  async function buildFinder(opts){
    opts=opts||{};
    const root=document.createElement("div");
    root.className="app-layout";
    let currentPath=opts.path||"~/Desktop";
    let selected=null;
    root.innerHTML=`<aside class="app-sidebar">
      <div class="sidebar-section">Favorites</div>
      ${["~/Desktop","~/Documents","~/Downloads","~/Pictures","~/Music"].map(p=>`<button class="sidebar-item" data-path="${p}">${p.split("/").pop()}</button>`).join("")}
    </aside>
    <div class="app-main" style="display:flex;flex-direction:column">
      <div class="app-toolbar">
        <button class="tb-btn" id="f-up">◀</button>
        <strong style="font-size:13px" id="f-label">Desktop</strong>
        <button class="tb-btn" id="f-newf" style="margin-left:auto">📁+</button>
        <button class="tb-btn" id="f-new">📄+</button>
        <button class="tb-btn" id="f-del">🗑</button>
      </div>
      <div class="finder-files" id="f-files" style="flex:1;overflow:auto"></div>
      <div style="padding:6px 12px;font-size:11px;color:#6e6e73;border-top:.5px solid rgba(0,0,0,.08)" id="f-status"></div>
    </div>`;
    const grid=root.querySelector("#f-files");
    async function load(path){
      currentPath=path; selected=null;
      root.querySelector("#f-label").textContent=path.replace(/^~\//,"")||"Home";
      root.querySelectorAll(".sidebar-item").forEach(el=>el.classList.toggle("active",el.dataset.path===path));
      const r=await fetch("/api/fs/list?path="+encodeURIComponent(path));
      const data=await r.json();
      if(data.error){ root.querySelector("#f-status").textContent=data.error; return; }
      const entries=data.entries||[];
      root.querySelector("#f-status").textContent=`${entries.length} items — ${path}`;
      grid.innerHTML="";
      entries.forEach(e=>{
        const el=document.createElement("div");
        el.className="finder-file";
        el.innerHTML=e.kind==="dir"?`<div class="folder-icon"></div><span>${esc(e.name)}</span>`:`<div class="doc-icon">📄</div><span>${esc(e.name)}</span>`;
        el.onclick=()=>{grid.querySelectorAll(".finder-file").forEach(f=>f.classList.remove("selected"));el.classList.add("selected");selected=e};
        el.ondblclick=()=>{
          if(e.kind==="dir") load(e.path);
          else if(/\.(txt|md|json|csv|log|rs|js|css|html)$/i.test(e.name)||!e.name.includes(".")) openApp("textedit",{path:e.path});
          else openApp("preview");
        };
        grid.appendChild(el);
      });
    }
    root.querySelectorAll(".sidebar-item").forEach(i=>i.onclick=()=>load(i.dataset.path));
    root.querySelector("#f-up").onclick=()=>{
      const p=currentPath.replace(/\/$/,"").split("/"); p.pop();
      load(p.join("/")||"~");
    };
    root.querySelector("#f-newf").onclick=async()=>{
      const name=prompt("Folder name:","Untitled Folder"); if(!name) return;
      await fetch("/api/fs/mkdir",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({path:currentPath.replace(/\/$/,"")+"/"+name})});
      load(currentPath);
    };
    root.querySelector("#f-new").onclick=async()=>{
      const name=prompt("File name:","Untitled.txt"); if(!name) return;
      const path=currentPath.replace(/\/$/,"")+"/"+name;
      await fetch("/api/fs/create",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({path,content:""})});
      load(currentPath); openApp("textedit",{path});
    };
    root.querySelector("#f-del").onclick=async()=>{
      if(!selected||!confirm("Delete "+selected.name+"?")) return;
      await fetch("/api/fs/delete?path="+encodeURIComponent(selected.path),{method:"DELETE"});
      load(currentPath);
    };
    await load(currentPath);
    return root;
  }

  async function buildNotes(opts){
    opts=opts||{};
    const root=document.createElement("div");
    root.className="notes-layout";
    root.innerHTML=`<div style="display:flex;flex-direction:column;width:220px;border-right:.5px solid rgba(0,0,0,.08)">
      <div class="notes-toolbar"><button class="tb-btn" id="n-new">✏️</button><button class="tb-btn" id="n-del">🗑</button></div>
      <div class="notes-list" id="n-list"></div></div>
      <div class="notes-editor"><input id="n-title" placeholder="Title"/><textarea id="n-body"></textarea></div>`;
    let notes=[], activeId=opts.noteId||null, timer=null;
    async function load(){ notes=await (await fetch("/api/notes")).json(); render(); if(activeId) select(activeId); else if(notes[0]) select(notes[0].id); }
    function render(){
      root.querySelector("#n-list").innerHTML=notes.map(n=>`<div class="note-item${n.id===activeId?" active":""}" data-id="${n.id}"><h4>${esc(n.title)}</h4><p>${esc(n.updated)}</p></div>`).join("");
      root.querySelectorAll(".note-item").forEach(el=>el.onclick=()=>select(el.dataset.id));
    }
    function select(id){ activeId=id; const n=notes.find(x=>x.id===id); if(!n)return; root.querySelector("#n-title").value=n.title; root.querySelector("#n-body").value=n.body; render(); }
    function schedule(){ clearTimeout(timer); timer=setTimeout(async()=>{
      if(!activeId)return;
      const r=await fetch("/api/notes/"+activeId,{method:"PUT",headers:{"Content-Type":"application/json"},body:JSON.stringify({title:root.querySelector("#n-title").value,body:root.querySelector("#n-body").value})});
      if(r.ok){ const u=await r.json(); const i=notes.findIndex(n=>n.id===activeId); if(i>=0) notes[i]=u; render(); }
    },400); }
    root.querySelector("#n-title").oninput=schedule; root.querySelector("#n-body").oninput=schedule;
    root.querySelector("#n-new").onclick=async()=>{ const n=await (await fetch("/api/notes",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({title:"New Note",body:""})})).json(); notes.unshift(n); select(n.id); g.MaxcosDesktop?.refreshNotifications?.(); };
    root.querySelector("#n-del").onclick=async()=>{ if(!activeId)return; await fetch("/api/notes/"+activeId,{method:"DELETE"}); notes=notes.filter(n=>n.id!==activeId); if(notes[0]) select(notes[0].id); else {activeId=null;render();} };
    await load(); return root;
  }

  async function buildTextEdit(opts){
    opts=opts||{};
    const root=document.createElement("div");
    root.className="textedit-body";
    let filePath=opts.path||null;
    root.innerHTML=`<div class="app-toolbar"><button class="tb-btn" id="te-save">💾</button><button class="tb-btn" id="te-saveas">Save As</button>
      <span style="font-size:12px;color:#6e6e73" id="te-path">${esc(filePath||"Untitled")}</span>
      <span style="margin-left:auto;font-size:11px;color:#6e6e73" id="te-st"></span></div>
      <textarea id="te-body"></textarea>`;
    const ta=root.querySelector("#te-body");
    if(filePath){
      const d=await (await fetch("/api/fs/read?path="+encodeURIComponent(filePath))).json();
      if(!d.error){ ta.value=d.content||""; filePath=d.path; root.querySelector("#te-path").textContent=filePath; }
    }else ta.value="New document — save to server disk.\n";
    async function saveTo(path){
      const d=await (await fetch("/api/fs/write",{method:"PUT",headers:{"Content-Type":"application/json"},body:JSON.stringify({path,content:ta.value})})).json();
      if(d.error){ root.querySelector("#te-st").textContent=d.error; return; }
      filePath=d.path; root.querySelector("#te-path").textContent=filePath; root.querySelector("#te-st").textContent="Saved";
      g.MaxcosDesktop?.refreshNotifications?.();
    }
    root.querySelector("#te-save").onclick=async()=>{ if(!filePath){ const p=prompt("Path:","~/Documents/Untitled.txt"); if(!p)return; filePath=p;} await saveTo(filePath); };
    root.querySelector("#te-saveas").onclick=async()=>{ const p=prompt("Save as:",filePath||"~/Documents/Untitled.txt"); if(p) await saveTo(p); };
    return root;
  }

  function buildCalculator(){
    const root=document.createElement("div"); root.className="calc-body";
    let display="0", expr="", just=false;
    root.innerHTML=`<div class="calc-display" id="cd">0</div><div class="calc-grid" id="cg"></div>`;
    const disp=root.querySelector("#cd"); const grid=root.querySelector("#cg");
    [["AC","fn"],["±","fn"],["%","fn"],["÷","op"],["7",""],["8",""],["9",""],["×","op"],["4",""],["5",""],["6",""],["−","op"],["1",""],["2",""],["3",""],["+","op"],["0","zero"],[".",""],["=","op"]].forEach(([lab,cls])=>{
      const b=document.createElement("button"); b.className="calc-btn "+cls; b.textContent=lab;
      b.onclick=async()=>{
        if(lab==="AC"){expr="";display="0";just=false;disp.textContent=display;return}
        if(lab==="="){
          const full=(expr+display).replace(/×/g,"*").replace(/÷/g,"/").replace(/−/g,"-");
          const d=await (await fetch("/api/calc?expr="+encodeURIComponent(full))).json();
          display=d.ok?String(d.result):"Error"; expr=""; just=true; disp.textContent=display; return;
        }
        if(["+","−","×","÷"].includes(lab)){expr=display+lab;just=true;return}
        if(just){display=lab==="."?"0.":lab;just=false} else if(display==="0"&&lab!==".") display=lab; else { if(lab==="."&&display.includes("."))return; display+=lab; }
        disp.textContent=display;
      };
      grid.appendChild(b);
    });
    return root;
  }

  async function buildReminders(){
    const root=document.createElement("div"); root.className="rem-body";
    let items=await (await fetch("/api/reminders")).json();
    function render(){
      root.innerHTML=`<h2>Reminders</h2><div>${items.map(r=>`<div class="rem-item${r.done?" done":""}" data-id="${r.id}">
        <div class="rem-check${r.done?" done":""}">${r.done?"✓":""}</div><span style="flex:1">${esc(r.text)}</span>
        <button class="tb-btn rem-x">×</button></div>`).join("")}</div>
        <div class="rem-add"><input id="r-in" placeholder="New Reminder"/><button id="r-add">Add</button></div>`;
      root.querySelectorAll(".rem-check").forEach(c=>c.onclick=async()=>{
        const id=c.closest(".rem-item").dataset.id;
        const u=await (await fetch("/api/reminders/"+id+"/toggle",{method:"POST"})).json();
        const i=items.findIndex(x=>x.id===id); if(i>=0) items[i]=u; render();
      });
      root.querySelectorAll(".rem-x").forEach(b=>b.onclick=async()=>{
        const id=b.closest(".rem-item").dataset.id;
        await fetch("/api/reminders/"+id,{method:"DELETE"}); items=items.filter(x=>x.id!==id); render();
      });
      root.querySelector("#r-add").onclick=add;
      root.querySelector("#r-in").onkeydown=e=>{if(e.key==="Enter")add()};
      async function add(){
        const t=root.querySelector("#r-in").value.trim(); if(!t)return;
        const r=await (await fetch("/api/reminders",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({text:t})})).json();
        items.push(r); render(); g.MaxcosDesktop?.refreshNotifications?.();
      }
    }
    render(); return root;
  }


  function buildSettings(){
    const root=document.createElement("div"); root.className="settings-layout";
    const panes = {
      General: () => `<h2>General</h2><div class="settings-card">
        <div class="settings-row"><span>Account</span><span style="color:#6e6e73">${esc(g.MAXCOS.username||"User")}</span></div>
        <div class="settings-row"><span>User ID</span><span style="color:#6e6e73;font-size:11px">${esc(g.MAXCOS.userId||"")}</span></div>
        <div class="settings-row"><span>Appearance</span><span style="color:#6e6e73">Auto</span></div>
      </div>`,
      Desktop: () => {
        const walls = ["sonoma","sequoia","midnight","dawn"];
        const cur = g.MaxcosWM.getSpaces().find(s=>s.id===g.MaxcosWM.getActiveSpaceId())?.wallpaper || "sonoma";
        return `<h2>Desktop & Dock</h2><div class="settings-card">
          <div class="settings-row"><span>Current Space wallpaper</span></div>
          <div style="display:flex;gap:10px;padding:12px;flex-wrap:wrap">
            ${walls.map(w=>`<button type="button" class="wp-pick" data-wp="${w}" style="width:72px;height:48px;border-radius:8px;border:2px solid ${w===cur?"#0A84FF":"transparent"};overflow:hidden;padding:0">
              <div class="wallpaper wallpaper-${w}" style="width:100%;height:100%;position:relative"></div></button>`).join("")}
          </div>
          <div class="settings-row"><span>Spaces</span><span style="color:#6e6e73">${g.MaxcosWM.getSpaces().length} desktops</span></div>
        </div>`;
      },
      Users: () => `<h2>Users & Groups</h2>
        <div id="users-pane"><p style="color:#6e6e73">Loading…</p></div>
        <div class="settings-card" style="margin-top:12px;padding:12px">
          <strong style="display:block;margin-bottom:8px">New User</strong>
          <div style="display:flex;flex-wrap:wrap;gap:8px;align-items:center">
            <input id="nu-name" placeholder="Full name" style="padding:6px 10px;border-radius:6px;border:1px solid rgba(0,0,0,.12);flex:1;min-width:120px"/>
            <input id="nu-avatar" placeholder="Avatar (letter)" maxlength="2" style="width:70px;padding:6px 8px;border-radius:6px;border:1px solid rgba(0,0,0,.12)"/>
            <input id="nu-color" type="color" value="#0A84FF" style="width:40px;height:32px;border:none"/>
            <input id="nu-pass" type="password" placeholder="Password" required style="padding:6px 10px;border-radius:6px;border:1px solid rgba(0,0,0,.12);flex:1;min-width:120px"/>
            <button type="button" class="btn-primary" id="nu-create" style="padding:6px 14px">Create</button>
          </div>
        </div>
        <p style="font-size:12px;color:#6e6e73;margin-top:12px">Each user has a private home folder, notes, Safari data, and Spaces. Switch User from the Apple menu.</p>`,
      Admin: () => `<h2>Admin</h2>
        <p style="font-size:12px;color:#6e6e73;margin:0 0 12px">Security audit log from MongoDB (login, users, settings). Signed-in users only.</p>
        <div class="settings-card" style="padding:8px 12px;display:flex;justify-content:space-between;align-items:center">
          <span style="font-size:13px;color:#6e6e73">Source: maxcos.audit_log</span>
          <button type="button" class="tb-btn" id="audit-refresh" style="padding:4px 10px">Refresh</button>
        </div>
        <div id="audit-pane"><p style="color:#6e6e73">Loading…</p></div>`
    };
    const keys = Object.keys(panes);
    root.innerHTML = `<aside class="settings-nav">${keys.map((k,i)=>`<button class="sidebar-item${i===0?" active":""}" data-pane="${k}">${k}</button>`).join("")}</aside>
      <div class="settings-content" id="settings-content">${panes.General()}</div>`;

    async function loadUsersPane(){
      const box = root.querySelector("#users-pane");
      if(!box) return;
      try{
        const data = await (await fetch("/api/users")).json();
        const me = g.MAXCOS.userId;
        box.innerHTML = `<div class="settings-card">${(data.users||[]).map(u=>`
          <div class="settings-row">
            <span style="display:flex;align-items:center;gap:10px">
              <span style="width:28px;height:28px;border-radius:50%;background:${u.color};display:grid;place-items:center;color:#fff;font-size:12px;font-weight:600">${esc(u.avatar)}</span>
              <span>${esc(u.name)}${u.id===me?" (you)":""}</span>
            </span>
            ${u.id===me?`<span style="color:#6e6e73">Signed in</span>`:
              `<button type="button" class="tb-btn user-del" data-id="${u.id}" style="color:#ff3b30">Delete</button>`}
          </div>`).join("")}</div>`;
        box.querySelectorAll(".user-del").forEach(btn=>{
          btn.onclick = async ()=>{
            if(!confirm("Delete this user and all their data?")) return;
            const r = await fetch("/api/users/"+btn.dataset.id,{method:"DELETE"});
            if(!r.ok){ alert(await r.text()); return; }
            loadUsersPane();
            g.MaxcosDesktop?.toast?.("Users","Account deleted");
          };
        });
      }catch(e){ box.innerHTML = `<p style="color:#ff3b30">${esc(String(e))}</p>`; }
    }

    const actionColor = {
      login_success: "#34c759",
      login_fail: "#ff3b30",
      user_create: "#0A84FF",
      user_delete: "#ff9f0a",
      settings_change: "#5e5ce6",
    };

    async function loadAuditPane(){
      const box = root.querySelector("#audit-pane");
      if(!box) return;
      try{
        const data = await (await fetch("/api/admin/audit?limit=100")).json();
        const entries = data.entries || [];
        if(!entries.length){
          box.innerHTML = `<div class="settings-card"><div class="settings-row"><span style="color:#6e6e73">No audit events yet.</span></div></div>`;
          return;
        }
        box.innerHTML = `<div class="settings-card" style="max-height:360px;overflow:auto">${entries.map(e=>{
          const c = actionColor[e.action] || "#6e6e73";
          return `<div class="settings-row" style="align-items:flex-start;gap:12px">
            <span style="min-width:110px;font-size:11px;font-weight:600;color:${c}">${esc(e.action||"")}</span>
            <span style="flex:1;font-size:12px">
              <strong>${esc(e.username||e.user_id||"—")}</strong>
              <span style="color:#6e6e73"> · ${esc(e.detail||"")}</span>
              <div style="font-size:11px;color:#8e8e93;margin-top:2px">${esc(e.time||"")}</div>
            </span>
          </div>`;
        }).join("")}</div>`;
      }catch(e){
        box.innerHTML = `<p style="color:#ff3b30">${esc(String(e))}</p>`;
      }
    }

    function bindPane(name){
      const content = root.querySelector("#settings-content");
      content.innerHTML = panes[name]();
      if(name==="Users"){
        loadUsersPane();
        content.querySelector("#nu-create")?.addEventListener("click", async ()=>{
          const name = content.querySelector("#nu-name").value.trim();
          if(!name) return alert("Name required");
          const password = content.querySelector("#nu-pass").value;
          if(!password || password.length < 8) return alert("Password must be at least 8 characters");
          if(!/\d/.test(password)) return alert("Password must contain at least one number");
          const body = {
            name,
            avatar: content.querySelector("#nu-avatar").value.trim() || name[0],
            color: content.querySelector("#nu-color").value,
            password,
            password_hint: "",
            sign_in: false
          };
          const r = await fetch("/api/users",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(body)});
          if(!r.ok){ alert(await r.text()); return; }
          content.querySelector("#nu-name").value="";
          content.querySelector("#nu-pass").value="";
          loadUsersPane();
          g.MaxcosDesktop?.toast?.("Users","Created "+name);
        });
      }
      if(name==="Admin"){
        loadAuditPane();
        content.querySelector("#audit-refresh")?.addEventListener("click", loadAuditPane);
      }
      if(name==="Desktop"){
        content.querySelectorAll(".wp-pick").forEach(btn=>{
          btn.onclick = ()=>{
            const wp = btn.dataset.wp;
            const spaces = g.MaxcosWM.getSpaces();
            const active = g.MaxcosWM.getActiveSpaceId();
            const sp = spaces.find(s=>s.id===active);
            if(sp) sp.wallpaper = wp;
            g.MaxcosWM.setSpaces(spaces, active);
            g.MaxcosDesktop?.persistSettings?.();
            bindPane("Desktop");
          };
        });
      }
    }
    root.querySelectorAll(".sidebar-item").forEach(item=>{
      item.onclick = ()=>{
        root.querySelectorAll(".sidebar-item").forEach(i=>i.classList.remove("active"));
        item.classList.add("active");
        bindPane(item.dataset.pane);
      };
    });
    return root;
  }


  async function buildMail(){
    const root=document.createElement("div"); root.className="mail-layout";
    const data=await (await fetch("/api/mail")).json();
    let active=data.inbox[0];
    function render(){
      root.innerHTML=`<div class="mail-list">${data.inbox.map(m=>`<div class="mail-item${m.id===active.id?" active":""}" data-id="${m.id}">
        <div><strong>${esc(m.from)}</strong> <span style="float:right;font-size:11px;color:#6e6e73">${m.time}</span></div>
        <div>${esc(m.subject)}</div><div style="font-size:12px;color:#6e6e73">${esc(m.preview)}</div></div>`).join("")}</div>
        <div class="mail-read"><h2>${esc(active.subject)}</h2><div class="meta" style="color:#6e6e73;margin-bottom:16px">${esc(active.from)}</div><div>${esc(active.body)}</div></div>`;
      root.querySelectorAll(".mail-item").forEach(el=>el.onclick=()=>{active=data.inbox.find(m=>m.id===el.dataset.id);render()});
    }
    render(); return root;
  }
  async function buildMessages(){
    const root=document.createElement("div"); root.className="msg-layout";
    const data=await (await fetch("/api/messages")).json();
    let active=data.conversations[0];
    function render(){
      root.innerHTML=`<div class="msg-list">${data.conversations.map(c=>`<div class="msg-conv${c.id===active.id?" active":""}" data-id="${c.id}">
        <div class="msg-avatar" style="background:${c.color}">${c.avatar}</div><div><strong>${esc(c.name)}</strong><div style="font-size:12px;color:#6e6e73">${esc(c.last)}</div></div></div>`).join("")}</div>
        <div class="msg-thread"><div style="padding:12px;font-weight:600;border-bottom:.5px solid rgba(0,0,0,.08)">${esc(active.name)}</div>
        <div class="msg-bubbles">${active.messages.map(m=>`<div class="bubble ${m.from}">${esc(m.text)}</div>`).join("")}</div>
        <div class="msg-compose"><input id="m-in" placeholder="iMessage"/><button id="m-send">↑</button></div></div>`;
      root.querySelectorAll(".msg-conv").forEach(el=>el.onclick=()=>{active=data.conversations.find(c=>c.id===el.dataset.id);render()});
      const send=()=>{const t=root.querySelector("#m-in").value.trim();if(!t)return;active.messages.push({from:"me",text:t});active.last=t;render()};
      root.querySelector("#m-send").onclick=send; root.querySelector("#m-in").onkeydown=e=>{if(e.key==="Enter")send()};
    }
    render(); return root;
  }
  async function buildPhotos(){
    const data=await (await fetch("/api/photos")).json();
    const root=document.createElement("div"); root.className="app-main";
    root.innerHTML=`<div class="photos-grid">${data.photos.map(p=>`<div class="photo-thumb" style="background:${p.gradient}">${esc(p.title)}</div>`).join("")}</div>`;
    return root;
  }
  async function buildMusic(){
    const data=await (await fetch("/api/music")).json();
    const root=document.createElement("div"); root.className="music-layout";
    const t=data.tracks[0];
    root.innerHTML=`<div class="app-main"><div class="music-now"><div style="font-size:40px">♪</div><div><strong>${esc(t.title)}</strong><div>${esc(t.artist)}</div></div></div>
      ${data.tracks.map((x,i)=>`<div class="track-row"><span>${i+1}</span><div style="flex:1"><strong>${esc(x.title)}</strong><div style="font-size:12px;color:#6e6e73">${esc(x.artist)}</div></div><span>${x.duration}</span></div>`).join("")}</div>`;
    return root;
  }
  async function buildCalendar(){
    const data=await (await fetch("/api/calendar")).json();
    const root=document.createElement("div"); root.style.height="100%";
    const months=["January","February","March","April","May","June","July","August","September","October","November","December"];
    const first=new Date(data.year,data.month-1,1).getDay();
    const dim=new Date(data.year,data.month,0).getDate();
    let cells=""; for(let i=0;i<first;i++) cells+=`<div class="cal-day"></div>`;
    for(let d=1;d<=dim;d++) cells+=`<div class="cal-day${d===data.day?" today":""}"><span class="day-num">${d}</span></div>`;
    root.innerHTML=`<div class="cal-header"><h2 style="margin:0">${months[data.month-1]} ${data.year}</h2></div>
      <div class="cal-grid">${["S","M","T","W","T","F","S"].map(d=>`<div class="cal-dow">${d}</div>`).join("")}${cells}</div>`;
    return root;
  }
  function buildMaps(){
    const root=document.createElement("div"); root.className="maps-body";
    root.innerHTML=`<div class="maps-search"><input placeholder="Search Maps" id="mq"/></div><div class="maps-pin">📍</div>
      <div style="position:absolute;top:48%;left:50%;transform:translateX(-50%);background:#fff;padding:6px 12px;border-radius:8px;font-size:12px;box-shadow:0 2px 8px rgba(0,0,0,.15)" id="ml">Apple Park</div>`;
    root.querySelector("#mq").onkeydown=e=>{if(e.key==="Enter") root.querySelector("#ml").textContent=e.target.value||"Apple Park"};
    return root;
  }
  function buildFaceTime(){
    const root=document.createElement("div"); root.className="facetime-body";
    root.innerHTML=`<div style="font-size:64px;margin-bottom:12px">👤</div><div>Ready to call</div>
      <button class="tb-btn" style="margin-top:20px;background:#ff3b30;color:#fff;width:56px;height:56px;border-radius:50%" id="ft-end">📵</button>`;
    root.querySelector("#ft-end").onclick=()=>{const w=root.closest(".window"); if(w) g.MaxcosWM.closeWindow(w)};
    return root;
  }
  function buildClock(){
    const root=document.createElement("div"); root.className="clock-body";
    root.innerHTML=`<div id="clk" style="font-size:48px;font-weight:200;font-variant-numeric:tabular-nums"></div>`;
    const tick=()=>{if(!root.isConnected)return; root.querySelector("#clk").textContent=new Date().toLocaleTimeString()};
    tick(); setInterval(tick,1000); return root;
  }
  function buildWeather(){
    const root=document.createElement("div"); root.className="weather-body";
    root.innerHTML=`<div>San Francisco</div><div class="weather-temp">72°</div><div>Partly Cloudy</div>`;
    return root;
  }
  function buildContacts(){
    const contacts=[{name:"Tim Cook",email:"tim@apple.com",color:"#0A84FF"},{name:"Mom",email:"mom@icloud.com",color:"#FF2D55"}];
    let active=contacts[0];
    const root=document.createElement("div"); root.className="contacts-layout";
    function render(){
      root.innerHTML=`<div class="contacts-list">${contacts.map(c=>`<div class="contact-item${c.name===active.name?" active":""}" data-n="${esc(c.name)}">
        <div class="msg-avatar" style="background:${c.color};width:32px;height:32px">${c.name[0]}</div><strong style="font-size:13px">${esc(c.name)}</strong></div>`).join("")}</div>
        <div class="contact-detail" style="padding:32px"><div class="msg-avatar" style="background:${active.color};width:80px;height:80px;font-size:32px;margin-bottom:16px">${active.name[0]}</div>
        <h2 style="margin:0 0 12px">${esc(active.name)}</h2><div>${esc(active.email)}</div></div>`;
      root.querySelectorAll(".contact-item").forEach(el=>el.onclick=()=>{active=contacts.find(c=>c.name===el.dataset.n);render()});
    }
    render(); return root;
  }
  function buildBooks(){
    const root=document.createElement("div"); root.style.height="100%";root.style.overflow="auto";root.style.background="#f5f5f7";
    root.innerHTML=`<h2 style="padding:20px;margin:0">Library</h2><div class="media-grid">
      ${[{t:"The Innovators",c:"#ff9500"},{t:"Rust in Action",c:"#dea584"}].map(b=>`<div class="media-card"><div class="media-card-art" style="background:${b.c}">📘</div>
      <div style="padding:10px;font-size:12px"><strong>${esc(b.t)}</strong></div></div>`).join("")}</div>`;
    return root;
  }
  function buildPodcasts(){
    const root=document.createElement("div"); root.style.height="100%";root.style.overflow="auto";root.style.background="#f5f5f7";
    root.innerHTML=`<h2 style="padding:20px;margin:0">Podcasts</h2><div class="media-grid">
      <div class="media-card"><div class="media-card-art" style="background:#9933ff">🎙</div><div style="padding:10px;font-size:12px"><strong>ATP</strong></div></div></div>`;
    return root;
  }
  function buildTV(){
    const root=document.createElement("div"); root.style.height="100%";root.style.background="#000";root.style.color="#fff";root.style.overflow="auto";
    root.innerHTML=`<div style="padding:24px"><h2 style="margin:0 0 16px">Watch Now</h2>
      <div class="media-grid"><div class="media-card" style="background:#1c1c1e"><div class="media-card-art" style="background:linear-gradient(135deg,#2c3e50,#000);aspect-ratio:16/10">▶</div>
      <div style="padding:10px"><strong>Severance</strong></div></div></div></div>`;
    return root;
  }
  function buildAppStore(){
    const root=document.createElement("div"); root.className="as-body";
    root.innerHTML=`<div class="as-hero"><h2 style="margin:0 0 8px">Maxcos Pro</h2><p>Safari proxy · Real Terminal · Mission Control</p></div>
      <div style="padding:0 16px">${["Xcode","Pages","Keynote"].map(n=>`<div class="as-row"><div class="app-icon" style="width:56px;height:56px;background:#0a84ff;display:grid;place-items:center;color:#fff;font-weight:700">${n[0]}</div>
      <div style="flex:1"><strong>${n}</strong></div><button class="as-get">GET</button></div>`).join("")}</div>`;
    root.querySelectorAll(".as-get").forEach(b=>b.onclick=()=>{b.textContent="OPEN";pushNotification("Download Complete",b.closest(".as-row").querySelector("strong").textContent,"App Store","appstore")});
    return root;
  }
  function buildPreview(){
    const root=document.createElement("div"); root.className="preview-body";
    root.innerHTML=`<div class="preview-doc"><h2 style="margin-top:0">Preview</h2><p>Open documents from Finder.</p></div>`;
    return root;
  }
  function buildTrash(){
    const root=document.createElement("div"); root.className="trash-empty";
    root.innerHTML=`<div><div style="font-size:48px;opacity:.4">🗑</div><strong>Trash is Empty</strong></div>`;
    return root;
  }

  g.MaxcosApps={openApp,pushNotification};
})(window);
