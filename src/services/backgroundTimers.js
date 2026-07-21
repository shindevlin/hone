"use strict";

function shouldStartBackgroundTimers() {
  return process.env.NODE_ENV !== "test" && process.env.HONE_DISABLE_BACKGROUND_TIMERS !== "1";
}

module.exports = {
  shouldStartBackgroundTimers,
};
