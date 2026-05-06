"use strict";

jest.mock("../src/services/backgroundTimers", () => ({
  shouldStartBackgroundTimers: () => false,
}));

const serverInstances = [];

class MockServer {
  constructor(opts) {
    this.opts = opts;
    this.handlers = {};
    this.onceHandlers = {};
    this.closed = false;
    serverInstances.push(this);

    setImmediate(() => {
      if (opts.port === 6942 && !this._failedOnce) {
        this._failedOnce = true;
        this._emit("error", Object.assign(new Error("in use"), { code: "EADDRINUSE" }));
      } else {
        this._emit("listening");
      }
    });
  }

  on(event, handler) {
    (this.handlers[event] = this.handlers[event] || []).push(handler);
  }

  once(event, handler) {
    this.onceHandlers[event] = handler;
  }

  close() {
    this.closed = true;
  }

  _emit(event) {
    const args = Array.prototype.slice.call(arguments, 1);
    if (this.onceHandlers[event]) {
      const handler = this.onceHandlers[event];
      delete this.onceHandlers[event];
      handler.apply(this, args);
    }
    (this.handlers[event] || []).forEach((handler) => handler.apply(this, args));
  }
}

jest.mock("ws", () => {
  function MockWebSocket() {
    this.handlers = {};
    this.on = function (event, handler) {
      this.handlers[event] = handler;
    };
    this.close = jest.fn();
    setImmediate(() => {
      if (this.handlers.open) this.handlers.open();
    });
  }
  MockWebSocket.Server = MockServer;
  return MockWebSocket;
});

jest.mock("../src/p2p/protocol", () => ({
  handleMessage: jest.fn(),
  createHandshake: jest.fn(() => ({ type: "HANDSHAKE", data: {} })),
  knownPeers: new Set(),
  startPeerAnnounce: jest.fn(),
  stopPeerAnnounce: jest.fn(),
}));

describe("p2p network server retry", () => {
  beforeEach(() => {
    serverInstances.length = 0;
  });

  test("retry bind keeps connection handlers attached", async () => {
    const network = require("../src/p2p/network");
    network.startServer(6942);

    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(serverInstances.length).toBeGreaterThanOrEqual(2);
    const retryServer = serverInstances[serverInstances.length - 1];
    expect(retryServer.opts.port).toBe(6943);
    expect(Array.isArray(retryServer.handlers.connection)).toBe(true);
    expect(retryServer.handlers.connection.length).toBeGreaterThan(0);
    expect(Array.isArray(retryServer.handlers.error)).toBe(true);
    expect(retryServer.handlers.error.length).toBeGreaterThan(0);

    network.stopServer();
  });
});
