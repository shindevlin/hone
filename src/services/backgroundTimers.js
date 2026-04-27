"use strict";

function shouldStartBackgroundTimers() {
  return process.env.NODE_ENV !== "test" && process.env.BTCPC_DISABLE_BACKGROUND_TIMERS !== "1";
}

module.exports = {
  shouldStartBackgroundTimers,
};
