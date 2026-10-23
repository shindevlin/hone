"use strict";

/**
 * Sensor data HTTP route tests.
 *
 * Uses Node's built-in http + ephemeral express server. No supertest.
 * The route delegates billing math to sensorDataBilling; tests mock that
 * service so we only verify routing, auth, and response shaping.
 */

const http = require("http");
const express = require("express");

function makeTestServer(routeModule) {
  const app = express();
  app.use(express.json());
  app.use("/v1/sensor-data", routeModule.router || routeModule);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, path, body, user) {
  return new Promise((resolve, reject) => {
    const headers = { "Content-Type": "application/json" };
    if (user) headers["x-test-user"] = user;
    const data = body ? JSON.stringify(body) : null;
    if (data) headers["Content-Length"] = Buffer.byteLength(data);

    const req = http.request(
      { host: "127.0.0.1", port, method, path, headers },
      (res) => {
        let raw = "";
        res.on("data", (chunk) => (raw += chunk));
        res.on("end", () => {
          let parsed = null;
          try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on("error", reject);
    if (data) req.write(data);
    req.end();
  });
}

describe("sensor data HTTP routes", () => {
  let testServer;
  let port;

  afterEach(async () => {
    if (testServer) {
      await closeServer(testServer);
      testServer = null;
      port = null;
    }
  });

  test("returns 503 when the billing service is missing", async () => {
    jest.resetModules();
    jest.doMock("../src/middlewares/auth", () => ({
      authenticateToken: (req, res, next) => {
        const username = req.headers["x-test-user"];
        if (!username) return res.status(401).json({ error: "unauthenticated" });
        req.user = { username };
        next();
      },
    }));

    const routes = require("../src/routes/sensorDataRoutes");
    routes.__setBillingLoader(() => null);
    const serverTuple = await makeTestServer(routes);
    testServer = serverTuple.server;
    port = serverTuple.port;

    const r = await request(
      port,
      "POST",
      "/v1/sensor-data/quote",
      { type: "gps", region: "sv-san-salvador" },
      "consumer"
    );
    expect(r.status).toBe(503);
    expect(r.body.error.code).toBe("sensor_billing_unavailable");
  });

  test("delegates quote/query/rate-card to sensorDataBilling", async () => {
    const quoteSensorQuery = jest.fn(async (body, ctx) => ({
      quote_id: "sdq_123",
      body,
      ctx_account: ctx.account,
    }));
    const executePaidSensorQuery = jest.fn(async (body, ctx) => ({
      query_id: "sdqrun_123",
      body,
      ctx_account: ctx.account,
    }));
    const getRateCard = jest.fn(() => ({
      base_price_by_type: { gps: 1 },
      protocol_split_bps: 2000,
    }));

    jest.resetModules();
    jest.doMock("../src/middlewares/auth", () => ({
      authenticateToken: (req, res, next) => {
        const username = req.headers["x-test-user"];
        if (!username) return res.status(401).json({ error: "unauthenticated" });
        req.user = { username };
        next();
      },
    }));

    const routes = require("../src/routes/sensorDataRoutes");
    routes.__setBillingLoader(() => ({
      quoteSensorQuery,
      executePaidSensorQuery,
      getRateCard,
    }));
    const serverTuple = await makeTestServer(routes);
    testServer = serverTuple.server;
    port = serverTuple.port;

    const rateCard = await request(port, "GET", "/v1/sensor-data/rate-card");
    expect(rateCard.status).toBe(200);
    expect(rateCard.body.rate_card.protocol_split_bps).toBe(2000);
    expect(getRateCard).toHaveBeenCalledTimes(1);

    const quote = await request(
      port,
      "POST",
      "/v1/sensor-data/quote",
      { type: "gps", region: "sv-san-salvador", limit: 100 },
      "consumer"
    );
    expect(quote.status).toBe(200);
    expect(quote.body.quote.quote_id).toBe("sdq_123");
    expect(quoteSensorQuery).toHaveBeenCalledTimes(1);
    expect(quoteSensorQuery.mock.calls[0][1].account).toBe("consumer");

    const query = await request(
      port,
      "POST",
      "/v1/sensor-data/query",
      { quote_id: "sdq_123", type: "gps", region: "sv-san-salvador" },
      "consumer"
    );
    expect(query.status).toBe(200);
    expect(query.body.result.query_id).toBe("sdqrun_123");
    expect(executePaidSensorQuery).toHaveBeenCalledTimes(1);
    expect(executePaidSensorQuery.mock.calls[0][1].account).toBe("consumer");
  });
});
