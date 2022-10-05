function toggleMenu() {
  const menu = document.querySelector("#nav-menu");
  const hidden = "hidden";

  if (menu.classList.contains(hidden)) {
    menu.classList.remove(hidden);
    this.setAttribute("aria-expanded", "true");
  } else {
    this.setAttribute("aria-expanded", "false");
    menu.classList.add(hidden);
  }
}

document.addEventListener("DOMContentLoaded", () => {
  const toggle = document.querySelector("#nav-toggle");

  // Clear any old event handlers to make this idempotent. Without this, the
  // menu toggle stops working because it immediately re-hides after showing.
  //
  // TODO: Is this fixed now that Turbo(links) isn't involved?
  toggle.removeEventListener("click", toggleMenu);

  toggle.addEventListener("click", toggleMenu);
});
