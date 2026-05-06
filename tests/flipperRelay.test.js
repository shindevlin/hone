"use strict";

const http = require("http");
const { parseUrlList, postJsonToAny } = require("../src/services/flipperRelay");

function listen(server) {
  return new Promise(function (resolve, reject) {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", function () {
      var address = server.address();
      resolve(address.port);
    });
  });
}

describe("flipperRelay", function () {
  test("parseUrlList trims, splits, and deduplicates", function () {
    expect(parseUrlList(" http://a.test , http://b.test\nhttp://a.test  ")).toEqual([
      "http://a.test",
      "http://b.test",
    ]);
  });

  test("postJsonToAny fails over to the next relay target", async function () {
    var firstHits = 0;
    var secondHits = 0;
    var receivedBody = null;

    var firstServer = http.createServer(function (_req, res) {
      firstHits++;
      res.statusCode = 503;
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({ error: "down" }));
    });

    var secondServer = http.createServer(function (req, res) {
      secondHits++;
      var body = "";
      req.on("data", function (chunk) { body += chunk; });
      req.on("end", function () {
        receivedBody = body;
        res.statusCode = 201;
        res.setHeader("Content-Type", "application/json");
        res.end(JSON.stringify({ success: true }));
      });
    });

    var firstPort = await listen(firstServer);
    var secondPort = await listen(secondServer);

    try {
      var result = await postJsonToAny([
        "http://127.0.0.1:" + firstPort + "/relay",
        "http://127.0.0.1:" + secondPort + "/relay",
      ], { sensor: "josh/flipper-subghz", value: -81 });

      expect(result.status).toBe(201);
      expect(firstHits).toBe(1);
      expect(secondHits).toBe(1);
      expect(JSON.parse(receivedBody)).toEqual({
        sensor: "josh/flipper-subghz",
        value: -81,
      });
    } finally {
      await new Promise(function (resolve) { firstServer.close(resolve); });
      await new Promise(function (resolve) { secondServer.close(resolve); });
    }
  });
});
