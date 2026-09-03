import init, { health } from "./extension.js";

const state = document.getElementById("state");

init()
  .then(() => {
    const reported = JSON.parse(health());
    state.textContent = `shell ${reported.version} — ${reported.status}`;
  })
  .catch((failure) => {
    state.textContent = `shell unavailable: ${failure}`;
  });
