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

// Categories: drag a category at its handle into a different group or position.
// Without JavaScript, the group list of each category does the same.
(() => {
  const lists = document.querySelectorAll("ul.category-list[data-group-id]");
  if (lists.length === 0) {
    return;
  }
  document.documentElement.classList.add("js-dnd");
  const status = document.querySelector("[data-move-status]");
  let dragged = null;
  let origin = null;

  const itemAfter = (list, y) => {
    for (const item of list.querySelectorAll(":scope > li[data-category-id]")) {
      if (item === dragged) {
        continue;
      }
      const box = item.getBoundingClientRect();
      if (y < box.top + box.height / 2) {
        return item;
      }
    }
    return null;
  };

  const clearTargets = () => {
    for (const list of lists) {
      list.classList.remove("drop-target");
    }
  };

  const save = async (item) => {
    const list = item.parentElement;
    const next = item.nextElementSibling;
    const form = item.querySelector("form.move-form");
    const data = new FormData();
    data.set("csrf_token", form.elements.csrf_token.value);
    data.set("group_id", list.dataset.groupId);
    if (next) {
      data.set("before_id", next.dataset.categoryId);
    }
    try {
      const response = await fetch(form.action, {
        method: "POST",
        body: data,
        headers: { Accept: "application/json" },
        credentials: "same-origin",
      });
      if (!response.ok) {
        throw new Error(String(response.status));
      }
      const result = await response.json();
      form.elements.group_id.value = list.dataset.groupId;
      const name = item.querySelector("input[name=name]").value;
      if (status) {
        status.textContent = `"${name}" is in the group "${result.group}" now.`;
      }
    } catch {
      // Show the state of the server again.
      window.location.reload();
    }
  };

  document.addEventListener("dragstart", (event) => {
    const handle = event.target instanceof Element ? event.target.closest(".drag-handle") : null;
    if (!handle) {
      return;
    }
    dragged = handle.closest("li[data-category-id]");
    origin = { list: dragged.parentElement, next: dragged.nextElementSibling };
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", dragged.dataset.categoryId);
    event.dataTransfer.setDragImage(dragged, 12, 12);
    dragged.classList.add("dragging");
  });

  for (const list of lists) {
    list.addEventListener("dragover", (event) => {
      if (!dragged) {
        return;
      }
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const after = itemAfter(list, event.clientY);
      if (after === null) {
        if (list.lastElementChild !== dragged) {
          list.appendChild(dragged);
        }
      } else if (after.previousElementSibling !== dragged) {
        list.insertBefore(dragged, after);
      }
      clearTargets();
      list.classList.add("drop-target");
    });
    list.addEventListener("drop", (event) => {
      if (!dragged) {
        return;
      }
      event.preventDefault();
      const item = dragged;
      const moved = item.parentElement !== origin.list || item.nextElementSibling !== origin.next;
      item.classList.remove("dragging");
      dragged = null;
      clearTargets();
      if (moved) {
        save(item);
      }
    });
  }

  document.addEventListener("dragend", () => {
    if (!dragged) {
      return;
    }
    // The drop was not on a group: put the category back.
    origin.list.insertBefore(dragged, origin.next);
    dragged.classList.remove("dragging");
    dragged = null;
    clearTargets();
  });
})();

// Transactions: the bulk category form. "Select all matching transactions" also
// selects the checkboxes of the page. A change of one checkbox makes the selection
// manual again. The form shows the number of selected transactions, and asks for a
// confirmation before it changes all matching transactions. Without JavaScript, the
// form also works: the server ignores the checkboxes of the rows for "all".
(() => {
  const form = document.getElementById("bulk");
  if (!form) {
    return;
  }
  const all = form.querySelector("input[data-select-all]");
  const select = form.elements.category_id;
  const boxes = () => document.querySelectorAll('input[type=checkbox][name=tx_id][form="bulk"]');
  const count = form.querySelector("[data-selected-count]");
  const update = () => {
    const allSelected = all !== null && all.checked;
    const selected = allSelected
      ? Number(all.dataset.total)
      : [...boxes()].filter((box) => box.checked).length;
    if (count) {
      count.textContent = selected === 0 ? "" : `${selected} selected.`;
    }
    if (allSelected) {
      const option = select.options[select.selectedIndex];
      const name = select.value === "" ? "" : ` "${option.text.trim()}"`;
      form.dataset.confirm = `Give the category${name} to ${selected} transactions?`;
    } else {
      delete form.dataset.confirm;
    }
  };
  const selectRows = () => {
    for (const box of boxes()) {
      box.checked = all.checked;
    }
  };
  document.addEventListener("change", (event) => {
    const el = event.target;
    if (!(el instanceof HTMLInputElement || el instanceof HTMLSelectElement) || el.form !== form) {
      return;
    }
    if (all && el === all) {
      selectRows();
    } else if (all && el.name === "tx_id") {
      all.checked = false;
    }
    update();
  });
  // The browser can restore the state of the form, for example after "Back".
  if (all && all.checked) {
    selectRows();
  }
  update();
})();
