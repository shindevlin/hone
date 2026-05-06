import React, { useEffect, useMemo, useState } from "https://esm.sh/react@18.3.1";
import { createRoot } from "https://esm.sh/react-dom@18.3.1/client";
import htm from "https://esm.sh/htm@3.1.1";
import {
  PrivyProvider,
  useLogin,
  usePrivy,
  useSignMessage,
  useWallets,
} from "https://esm.sh/@privy-io/react-auth@latest?deps=react@18.3.1,react-dom@18.3.1";

const html = htm.bind(React.createElement);
const STORAGE = {
  appId: "btcpc-privy-app-id",
  clientId: "btcpc-privy-client-id",
  to: "btcpc-controller-to",
  amount: "btcpc-controller-amount",
  memo: "btcpc-controller-memo",
  selectedWallet: "btcpc-controller-wallet",
};

function readStorage(key, fallback = "") {
  try {
    return localStorage.getItem(key) || fallback;
  } catch (_) {
    return fallback;
  }
}

function writeStorage(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (_) {}
}

function apiHeaders() {
  const jwt = readStorage("btcpc-jwt") || readStorage("btcpc-token");
  const headers = { "Content-Type": "application/json" };
  if (jwt) headers.Authorization = `Bearer ${jwt}`;
  return headers;
}

async function api(path, options) {
  const res = await fetch(path, Object.assign({ headers: apiHeaders() }, options || {}));
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || res.statusText);
  return data;
}

function isEvmWallet(wallet) {
  const chainType = String(wallet && (wallet.chainType || wallet.walletType || wallet.type || "")).toLowerCase();
  return chainType.includes("ethereum") || chainType.includes("evm") || chainType === "eth";
}

function walletLabel(wallet) {
  if (!wallet) return "--";
  return `${String(wallet.chainType || wallet.type || "wallet").toUpperCase()} · ${wallet.address}`;
}

function App() {
  const [appId, setAppId] = useState(readStorage(STORAGE.appId));
  const [clientId, setClientId] = useState(readStorage(STORAGE.clientId));
  const [editingConfig, setEditingConfig] = useState(!appId);

  useEffect(() => {
    document.title = "BTCPC Controller Wallet";
  }, []);

  const saveConfig = () => {
    const nextAppId = appId.trim();
    const nextClientId = clientId.trim();
    if (!nextAppId) {
      setEditingConfig(true);
      return;
    }
    writeStorage(STORAGE.appId, nextAppId);
    writeStorage(STORAGE.clientId, nextClientId);
    setAppId(nextAppId);
    setClientId(nextClientId);
    setEditingConfig(false);
  };

  const controllerSection = appId && !editingConfig
    ? html`
        <PrivyProvider
          appId=${appId}
          clientId=${clientId || undefined}
          config=${{
            embeddedWallets: {
              ethereum: {
                createOnLogin: "users-without-wallets",
              },
            },
          }}
        >
          <ControllerPanel />
        </PrivyProvider>
      `
    : html`
        <div className="split">
          <div className="panel">
            <h2>What this page does</h2>
            <div className="list">
              <div className="item">
                <strong>1. Log in to Privy</strong>
                <span>Use the embedded wallet or link an existing EVM wallet.</span>
              </div>
              <div className="item">
                <strong>2. Request a BTCPC challenge</strong>
                <span>BTCPC returns the exact spend challenge that needs approval.</span>
              </div>
              <div className="item">
                <strong>3. Sign and submit</strong>
                <span>The controller wallet signs the challenge and BTCPC accepts it only if it matches the linked wallet.</span>
              </div>
            </div>
          </div>
        </div>
      `;

  return html`
    <div className="shell">
      <div className="topbar">
        <a className="brand" href="/">
          <img src="/favicon-32.png" alt="BTCPC" />
          <strong>BTCPC Controller Wallet</strong>
        </a>
        <div className="navlinks">
          <a href="/start">Start</a>
          <a href="/install">Install</a>
          <a href="/app">App</a>
          <a href="/docs/BTCPC_WHITEPAPER.md">Whitepaper</a>
          <a href="/terms">Terms</a>
          <a href="/privacy">Privacy</a>
        </div>
      </div>

      <div className="hero">
        <div className="panel">
          <div className="eyebrow">Privy-backed controller signing</div>
          <h1>
            Sign BTCPC spends with the <em>same linked wallet</em>.
          </h1>
          <p className="lede">
            This page is the first live controller lane for BTCPC. It uses a
            Privy-connected wallet to sign the BTCPC transfer challenge from
            the browser, then submits that approval back to BTCPC. The
            controller wallet remains the BTCPC mnemonic-derived wallet on the
            linked chain. Secondary approval stays separate. No BTCPC browser
            extension is required.
          </p>
        </div>

        <div className="panel stack">
          <div className="card">
            <h2>Privy setup</h2>
            <p>
              Enter your Privy app credentials once and the page will remember
              them locally.
            </p>
            <div className="grid" style={{ marginTop: "12px" }}>
              <div className="field">
                <label>Privy App ID</label>
                <input
                  value={appId}
                  onChange={(e) => setAppId(e.target.value)}
                  placeholder="your-privy-app-id"
                />
              </div>
              <div className="field">
                <label>Privy Client ID</label>
                <input
                  value={clientId}
                  onChange={(e) => setClientId(e.target.value)}
                  placeholder="optional-client-id"
                />
              </div>
            </div>
            <div className="row" style={{ marginTop: "12px" }}>
              <button className="btn" onClick={saveConfig}>
                {appId ? "Use Saved Config" : "Save Privy Config"}
              </button>
              <button className="btn secondary" onClick={() => setEditingConfig((v) => !v)}>
                {editingConfig ? "Hide Config" : "Edit Config"}
              </button>
            </div>
          </div>

          <div className="hint">
            Today’s live path is EVM controller signing through Privy. The same
            BTCPC policy engine can later expand this page to other supported
            chains without changing the transfer flow.
          </div>
        </div>
      </div>

      ${controllerSection}

      <div className="footer" style={{ marginTop: "28px", color: "var(--muted)", fontSize: "13px" }}>
        Version 2026-04-24-v7 ·{" "}
        <a href="/start" style={{ color: "var(--accent)", textDecoration: "none" }}>Start</a>
        {" · "}
        <a href="/terms" style={{ color: "var(--accent)", textDecoration: "none" }}>Terms</a>
        {" · "}
        <a href="/privacy" style={{ color: "var(--accent)", textDecoration: "none" }}>Privacy</a>
      </div>
    </div>
  `;
}

function ControllerPanel() {
  const { ready, authenticated, user, logout } = usePrivy();
  const { login } = useLogin({
    onComplete: () => {
      // Privy has finished authentication; wallet state will refresh from hooks.
    },
  });
  const { wallets } = useWallets();
  const { signMessage } = useSignMessage();

  const [policy, setPolicy] = useState(null);
  const [to, setTo] = useState(readStorage(STORAGE.to));
  const [amount, setAmount] = useState(readStorage(STORAGE.amount));
  const [memo, setMemo] = useState(readStorage(STORAGE.memo));
  const [selectedWallet, setSelectedWallet] = useState(readStorage(STORAGE.selectedWallet));
  const [challenge, setChallenge] = useState(null);
  const [signature, setSignature] = useState("");
  const [status, setStatus] = useState("");
  const [statusKind, setStatusKind] = useState("");

  const evmWallets = useMemo(() => (wallets || []).filter(isEvmWallet), [wallets]);
  const activeWallet =
    evmWallets.find((wallet) => wallet.address === selectedWallet) ||
    evmWallets[0] ||
    null;
  const walletOptions = !evmWallets.length
    ? html`<option value="">Connect an EVM wallet in Privy</option>`
    : evmWallets.map((wallet) => html`
        <option value=${wallet.address}>${walletLabel(wallet)}</option>
      `);

  useEffect(() => {
    const jwt = readStorage("btcpc-jwt") || readStorage("btcpc-token");
    if (!jwt) {
      setStatus("Log in to BTCPC first so the controller page can request a spend challenge.");
      setStatusKind("error");
      return;
    }
    api("/api/wallet/private-auth")
      .then((data) => {
        setPolicy(data.policy || null);
      })
      .catch((err) => {
        setStatus(err.message);
        setStatusKind("error");
      });
  }, []);

  useEffect(() => {
    if (activeWallet && activeWallet.address) {
      setSelectedWallet(activeWallet.address);
      writeStorage(STORAGE.selectedWallet, activeWallet.address);
    }
  }, [activeWallet && activeWallet.address]);

  useEffect(() => {
    writeStorage(STORAGE.to, to);
  }, [to]);

  useEffect(() => {
    writeStorage(STORAGE.amount, amount);
  }, [amount]);

  useEffect(() => {
    writeStorage(STORAGE.memo, memo);
  }, [memo]);

  const controllerChain = "evm";

  async function requestChallenge() {
    if (!to || !Number(amount) || Number(amount) <= 0) {
      throw new Error("Enter a recipient and a positive amount");
    }
    setStatus("Requesting BTCPC controller challenge...");
    setStatusKind("");
    const data = await api("/api/wallet/transfer/challenge", {
      method: "POST",
      body: JSON.stringify({
        toAddress: to,
        amount: Number(amount),
        token: "BTCPC",
        memo,
        approval_chain: controllerChain,
      }),
    });
    setChallenge(data.challenge || null);
    setSignature("");
    setStatus("Controller challenge ready.");
    setStatusKind("ok");
  }

  async function signChallenge() {
    if (!challenge || !challenge.challenge) {
      throw new Error("Request a challenge first");
    }
    if (!activeWallet || !activeWallet.address) {
      throw new Error("Connect the controller wallet first");
    }
    setStatus("Signing controller challenge with Privy...");
    setStatusKind("");
    const result = await signMessage(
      { message: challenge.challenge },
      {
        address: activeWallet.address,
        uiOptions: {
          title: "Sign BTCPC controller challenge",
        },
      },
    );
    setSignature(result.signature);
    setStatus("Signature captured.");
    setStatusKind("ok");
    return result.signature;
  }

  async function submitApproval() {
    if (!signature) {
      throw new Error("Sign the challenge first");
    }
    if (!challenge || !challenge.challengeId) {
      throw new Error("Request a challenge first");
    }
    setStatus("Submitting controller approval to BTCPC...");
    setStatusKind("");
    const data = await api("/api/wallet/transfer", {
      method: "POST",
      body: JSON.stringify({
        toAddress: to,
        amount: Number(amount),
        token: "BTCPC",
        memo,
        chain_auth: {
          chain: controllerChain,
          approval_chain: controllerChain,
          challengeId: challenge.challengeId,
          challenge: challenge.challenge,
          signature,
          address: activeWallet && activeWallet.address ? activeWallet.address : undefined,
        },
      }),
    });
    setStatus(`Spend queued. TX ${data.txHash || data.transaction?.txHash || "--"}`);
    setStatusKind("ok");
    return data;
  }

  return html`
    <div className="split">
      <div className="panel">
        <h2>Privy wallet state</h2>
        <div className="row" style={{ marginBottom: "12px" }}>
          <button className="btn" onClick={() => login()} disabled={!ready}>
            ${authenticated ? "Reconnect Privy wallet" : "Log in with Privy"}
          </button>
          <button className="btn secondary" onClick={() => logout?.()} disabled={!authenticated}>
            Log out
          </button>
        </div>
        <div className=${`status ${statusKind}`}>${status}</div>
        <div className="list" style={{ marginTop: "10px" }}>
          <div className="item">
            <strong>Privy ready</strong>
            <span>${ready ? "yes" : "loading"}</span>
          </div>
          <div className="item">
            <strong>Authenticated</strong>
            <span>${authenticated ? "yes" : "no"}</span>
          </div>
          <div className="item">
            <strong>User</strong>
            <span>${user ? user.id : "--"}</span>
          </div>
          <div className="item">
            <strong>Connected wallets</strong>
            <span>${wallets && wallets.length ? wallets.length : 0}</span>
          </div>
        </div>
      </div>

      <div className="grid">
        <div className="card">
          <h2>Wallets</h2>
          <p>Select the controller wallet that matches the BTCPC mnemonic-derived wallet on the linked chain.</p>
          <div className="field" style={{ marginTop: "12px" }}>
            <label>Controller wallet</label>
            <select
              value={selectedWallet}
              onChange={(e) => setSelectedWallet(e.target.value)}
            >
              ${walletOptions}
            </select>
          </div>
          <div className="list" style={{ marginTop: "12px" }}>
            <div className="item">
              <strong>Approval chain</strong>
              <span>EVM controller signing is live. BTCPC verifies the connected wallet against the mnemonic-linked wallet.</span>
            </div>
            <div className="item">
              <strong>Policy mode</strong>
              <span>${policy && policy.controller ? `${policy.controller.mode || "linked_chain"} · ${policy.controller.provider || "privy"}` : "loading policy"}</span>
            </div>
          </div>
        </div>

        <div className="card">
          <h2>Transfer request</h2>
          <div className="grid" style={{ marginTop: "12px" }}>
            <div className="field">
              <label>Recipient</label>
              <input value={to} onChange={(e) => setTo(e.target.value)} placeholder="BTCPC username or address" />
            </div>
            <div className="field">
              <label>Amount</label>
              <input value={amount} onChange={(e) => setAmount(e.target.value)} inputMode="decimal" placeholder="0.00" />
            </div>
          </div>
          <div className="field" style={{ marginTop: "12px" }}>
            <label>Memo</label>
            <input value={memo} onChange={(e) => setMemo(e.target.value)} placeholder="Optional memo" />
          </div>
          <div className="row" style={{ marginTop: "12px" }}>
            <button className="btn" onClick={() => requestChallenge().catch((err) => { setStatus(err.message); setStatusKind("error"); })}>
              Request BTCPC challenge
            </button>
            <button className="btn secondary" onClick={() => signChallenge().catch((err) => { setStatus(err.message); setStatusKind("error"); })} disabled={!challenge || !activeWallet}>
              Sign with Privy
            </button>
            <button className="btn secondary" onClick={() => submitApproval().catch((err) => { setStatus(err.message); setStatusKind("error"); })} disabled={!signature}>
              Submit approval
            </button>
          </div>
        </div>
      </div>

      <div className="grid">
        <div className="card">
          <h2>Challenge</h2>
          <p>The challenge comes from BTCPC and is bound to the exact transfer.</p>
          <textarea readOnly value={challenge ? challenge.challenge : ""} placeholder="Request a challenge first" />
        </div>
        <div className="card">
          <h2>Signature</h2>
          <p>Privy signs the BTCPC challenge with the selected controller wallet.</p>
          <textarea
            value={signature}
            onChange={(e) => setSignature(e.target.value)}
            placeholder="Signature will appear here after signing"
          />
        </div>
      </div>
    </div>
  `;
}

createRoot(document.getElementById("controller-app")).render(html`<${App} />`);
