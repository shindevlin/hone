/* BTCPC onboarding chat widget — drop <script src="chat-widget.js"></script> anywhere */
(function () {
  "use strict";

  const API = "/public/agent-chat";
  const PLACEHOLDER = 'Ask anything — "how do I start on Windows?"';
  const WELCOME = "Hey! I'm the BTCPC setup assistant. What platform are you on, or what do you want to know?";

  function detectPlatform() {
    const ua = navigator.userAgent;
    if (/Windows/i.test(ua)) return "Windows";
    if (/Android/i.test(ua)) return "Android";
    if (/iPhone|iPad/i.test(ua)) return "iOS";
    if (/Mac/i.test(ua)) return "macOS";
    if (/Linux/i.test(ua)) return "Linux";
    return "";
  }

  function escapeHtml(str) {
    return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function renderMarkdown(text) {
    return text
      .replace(/```([\s\S]*?)```/g, (_, c) => `<pre><code>${escapeHtml(c.trim())}</code></pre>`)
      .replace(/`([^`]+)`/g, (_, c) => `<code>${escapeHtml(c)}</code>`)
      .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
      .replace(/\n/g, "<br>");
  }

  const style = document.createElement("style");
  style.textContent = `
    #btcpc-chat-fab {
      position: fixed; bottom: 24px; right: 24px; z-index: 9998;
      width: 52px; height: 52px; border-radius: 50%;
      background: #f7931a; border: none; cursor: pointer;
      box-shadow: 0 4px 18px rgba(247,147,26,.45);
      display: flex; align-items: center; justify-content: center;
      transition: transform .15s;
    }
    #btcpc-chat-fab:hover { transform: scale(1.08); }
    #btcpc-chat-fab svg { width: 24px; height: 24px; fill: #fff; }
    #btcpc-chat-panel {
      position: fixed; bottom: 88px; right: 24px; z-index: 9999;
      width: min(380px, calc(100vw - 32px));
      background: #1a2235; border: 1px solid #2a3550; border-radius: 16px;
      box-shadow: 0 8px 40px rgba(0,0,0,.55);
      display: flex; flex-direction: column; overflow: hidden;
      max-height: min(520px, calc(100vh - 120px));
      transition: opacity .15s, transform .15s;
    }
    #btcpc-chat-panel.hidden { opacity: 0; pointer-events: none; transform: translateY(10px); }
    #btcpc-chat-header {
      display: flex; align-items: center; gap: 10px;
      padding: 14px 16px; background: #0f172a;
      border-bottom: 1px solid #2a3550;
    }
    #btcpc-chat-header .dot {
      width: 8px; height: 8px; border-radius: 50%; background: #22c55e;
      box-shadow: 0 0 6px #22c55e;
    }
    #btcpc-chat-header span { color: #e2e8f0; font-weight: 700; font-size: 14px; flex: 1; }
    #btcpc-chat-close {
      background: none; border: none; cursor: pointer; color: #94a3b8;
      font-size: 18px; line-height: 1; padding: 0 2px;
    }
    #btcpc-chat-messages {
      flex: 1; overflow-y: auto; padding: 14px;
      display: flex; flex-direction: column; gap: 10px;
      font-family: -apple-system, system-ui, sans-serif; font-size: 14px; line-height: 1.55;
    }
    .btcpc-msg { max-width: 88%; padding: 10px 13px; border-radius: 12px; word-break: break-word; }
    .btcpc-msg.user { align-self: flex-end; background: rgba(247,147,26,.18); color: #ffd8a8; border-radius: 12px 12px 3px 12px; }
    .btcpc-msg.agent { align-self: flex-start; background: #0f172a; color: #e2e8f0; border-radius: 12px 12px 12px 3px; }
    .btcpc-msg code { background: rgba(255,255,255,.08); padding: 1px 5px; border-radius: 4px; font-size: 12px; }
    .btcpc-msg pre { background: #0b1020; border: 1px solid rgba(247,147,26,.18); border-radius: 8px; padding: 10px; overflow-x: auto; margin: 6px 0 0; }
    .btcpc-msg pre code { background: none; padding: 0; font-size: 12px; color: #d9f99d; }
    .btcpc-typing { color: #94a3b8; font-size: 13px; align-self: flex-start; padding: 4px 0; }
    #btcpc-chat-input-row {
      display: flex; gap: 8px; padding: 12px 14px;
      border-top: 1px solid #2a3550; background: #0f172a;
    }
    #btcpc-chat-input {
      flex: 1; background: #1a2235; border: 1px solid #2a3550; border-radius: 10px;
      color: #e2e8f0; padding: 9px 12px; font-size: 14px; outline: none; resize: none;
      font-family: inherit; line-height: 1.4; max-height: 100px;
    }
    #btcpc-chat-input:focus { border-color: rgba(247,147,26,.45); }
    #btcpc-chat-send {
      background: #f7931a; border: none; border-radius: 10px; padding: 0 14px;
      cursor: pointer; color: #fff; font-weight: 700; font-size: 14px;
      transition: opacity .15s;
    }
    #btcpc-chat-send:disabled { opacity: .4; cursor: not-allowed; }
  `;
  document.head.appendChild(style);

  const fab = document.createElement("button");
  fab.id = "btcpc-chat-fab";
  fab.title = "Chat with BTCPC assistant";
  fab.innerHTML = `<svg viewBox="0 0 24 24"><path d="M20 2H4a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2z"/></svg>`;

  const panel = document.createElement("div");
  panel.id = "btcpc-chat-panel";
  panel.className = "hidden";
  panel.innerHTML = `
    <div id="btcpc-chat-header">
      <div class="dot"></div>
      <span>BTCPC Assistant</span>
      <button id="btcpc-chat-close" title="Close">&times;</button>
    </div>
    <div id="btcpc-chat-messages"></div>
    <div id="btcpc-chat-input-row">
      <textarea id="btcpc-chat-input" rows="1" placeholder="${PLACEHOLDER}"></textarea>
      <button id="btcpc-chat-send">Send</button>
    </div>
  `;

  document.body.appendChild(fab);
  document.body.appendChild(panel);

  const messages = panel.querySelector("#btcpc-chat-messages");
  const input = panel.querySelector("#btcpc-chat-input");
  const sendBtn = panel.querySelector("#btcpc-chat-send");

  let opened = false;
  let busy = false;
  const platform = detectPlatform();

  function addMessage(role, html, isHtml) {
    const div = document.createElement("div");
    div.className = `btcpc-msg ${role}`;
    div.innerHTML = isHtml ? html : escapeHtml(html);
    messages.appendChild(div);
    messages.scrollTop = messages.scrollHeight;
    return div;
  }

  function open() {
    if (opened) return;
    opened = true;
    panel.classList.remove("hidden");
    if (!messages.children.length) {
      addMessage("agent", renderMarkdown(WELCOME), true);
    }
    input.focus();
  }

  function close() {
    opened = false;
    panel.classList.add("hidden");
  }

  fab.addEventListener("click", () => (opened ? close() : open()));
  panel.querySelector("#btcpc-chat-close").addEventListener("click", close);

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  });

  input.addEventListener("input", () => {
    input.style.height = "auto";
    input.style.height = Math.min(input.scrollHeight, 100) + "px";
  });

  sendBtn.addEventListener("click", send);

  async function send() {
    const text = input.value.trim();
    if (!text || busy) return;

    busy = true;
    sendBtn.disabled = true;
    input.value = "";
    input.style.height = "auto";

    addMessage("user", text, false);

    const typingEl = document.createElement("div");
    typingEl.className = "btcpc-typing";
    typingEl.textContent = "…";
    messages.appendChild(typingEl);
    messages.scrollTop = messages.scrollHeight;

    let agentEl = null;
    let accumulated = "";

    try {
      const res = await fetch(API, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ message: text, platform }),
      });

      if (!res.ok) throw new Error("Agent unavailable");

      typingEl.remove();
      agentEl = addMessage("agent", "", true);

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        const lines = buf.split("\n");
        buf = lines.pop();
        for (const line of lines) {
          if (!line.startsWith("data: ")) continue;
          const payload = line.slice(6);
          if (payload === "[DONE]") break;
          try {
            accumulated += JSON.parse(payload);
            agentEl.innerHTML = renderMarkdown(accumulated);
            messages.scrollTop = messages.scrollHeight;
          } catch (_) {}
        }
      }
    } catch (err) {
      typingEl.remove();
      addMessage("agent", "Sorry, I'm not available right now. Check that the node is running.", false);
    }

    busy = false;
    sendBtn.disabled = false;
    input.focus();
  }
})();
