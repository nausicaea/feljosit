// Feljosit: small progressive enhancements. The application also works without JavaScript.
"use strict";

document.addEventListener("change", (event) => {
  const el = event.target;
  if (el instanceof HTMLElement && el.hasAttribute("data-autosubmit-change") && el.form) {
    el.form.requestSubmit();
  }
});

document.addEventListener("submit", (event) => {
  const form = event.target;
  if (form instanceof HTMLFormElement && form.dataset.confirm) {
    if (!window.confirm(form.dataset.confirm)) {
      event.preventDefault();
    }
  }
});
